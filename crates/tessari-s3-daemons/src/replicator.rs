//! Replication (S3 ref 11 M4): the tasks this region's group hands out are versions to send to their destination
//! bucket. Each is sent through the destination region's S3 surface as a replica — PutObject, or DeleteObject for a
//! delete marker, carrying the source version id — signed as the user the source's configuration names as its Role,
//! so the destination decides on it as on any request. A sent version is stamped `COMPLETED`; one that cannot be sent
//! waits and is tried again, and is stamped `FAILED` when the group gives it up.

use std::io;
use std::time::Duration;

use bytes::Bytes;
use futures_util::Stream;
use tessari_s3_constants::{DAEMON_RUN_TIMEOUT_SECS, REPLICATION_IN_FLIGHT, REPLICATION_POLL_SECS};
use tessari_s3_core::auth::{SignedRequest, amz_date, sign_header, uri_encode};
use tessari_s3_core::authz::UserName;
use tessari_s3_core::objects::checksum::{ChecksumAlgorithm, Declared};
use tessari_s3_infrastructure::ReplicaClient;
use tessari_s3_storage::Storage;
use tessari_s3_storage::objects::{Content, ObjectReader, ObjectService, VersionRead};
use tessari_s3_storage::replication::Task;
use tessari_s3_types::{BucketName, ObjectKey, SecretKey};

use crate::Error;
use crate::daemon::{Daemon, DaemonFuture, Report};

/// The header a replica carries: the version id its source gave it.
const REPLICA_VERSION: &str = "x-tessaridb-replica-version-id";

fn failed(message: impl ToString) -> Error {
    Error::Run {
        daemon: "replicator",
        message: message.to_string(),
    }
}

/// Sends this region's versions to their destination buckets.
pub struct Replicator {
    storage: Storage,
    client: ReplicaClient,
}

impl Replicator {
    /// A replicator over `storage`, sending with `client`.
    #[must_use]
    pub const fn new(storage: Storage, client: ReplicaClient) -> Self {
        Self { storage, client }
    }

    /// Sends `task`'s version; `Ok` once the destination holds it (or the version is gone and there is nothing to
    /// send), `Err` with the reason otherwise.
    async fn send(&self, task: &Task) -> std::result::Result<(), String> {
        let source = BucketName::new(&task.bucket).map_err(|error| error.to_string())?;
        let destination = BucketName::new(&task.destination).map_err(|error| error.to_string())?;
        let key = ObjectKey::new(&task.key).map_err(|error| error.to_string())?;
        let buckets = self.storage.buckets();
        let role = buckets
            .get(&source)
            .await
            .map_err(|error| error.to_string())?
            .and_then(|bucket| bucket.replication)
            .map(|configuration| configuration.role)
            .ok_or("the source bucket no longer replicates")?;
        let region = buckets
            .get(&destination)
            .await
            .map_err(|error| error.to_string())?
            .map(|bucket| bucket.region)
            .ok_or("the destination bucket does not exist")?;
        let endpoint = self
            .storage
            .replication()
            .endpoint(&region)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("region {region} advertises no S3 endpoint"))?;
        let (access_key, secret) = self.credential(&role).await?;
        let mut path = format!("/{}/", destination.as_str());
        uri_encode(key.as_str().as_bytes(), true, &mut path);
        let host = host_of(&endpoint).ok_or("the advertised endpoint names no host")?;
        let date = amz_date(now_secs());
        let mut headers: Vec<(String, String)> = vec![
            ("host".to_owned(), host.to_owned()),
            ("x-amz-date".to_owned(), date),
            (
                "x-amz-content-sha256".to_owned(),
                "UNSIGNED-PAYLOAD".to_owned(),
            ),
            (REPLICA_VERSION.to_owned(), task.version_id.clone()),
        ];
        let objects = self.storage.objects();
        let (method, body) = if task.marker {
            ("DELETE", None)
        } else {
            let object = match objects
                .get_version(&source, &key, &task.version_id)
                .await
                .map_err(|error| error.to_string())?
            {
                VersionRead::Object(object) => object,
                // Removed by its id since it was queued: nothing is left to send.
                VersionRead::Absent => return Ok(()),
                other => return Err(format!("the version cannot be read: {other:?}")),
            };
            headers.push(("content-length".to_owned(), object.size.to_string()));
            for (name, value) in &object.headers {
                headers.push((name.clone(), value.clone()));
            }
            for (name, value) in &object.metadata {
                headers.push((format!("x-amz-meta-{name}"), value.clone()));
            }
            for (name, value) in &object.checksums {
                if let Some(Declared::Supported(algorithm)) = ChecksumAlgorithm::parse(name) {
                    headers.push((algorithm.header().to_owned(), value.clone()));
                }
            }
            let pieces = match &object.content {
                Content::Data(id) => vec![(*id, object.size)],
                Content::Parts(multipart) => multipart
                    .parts
                    .iter()
                    .map(|part| (part.data, part.size))
                    .collect(),
            };
            ("PUT", Some(blocks(objects.clone(), pieces)))
        };
        let signed: Vec<(&str, &str)> = headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        let authorization = sign_header(
            &SignedRequest {
                method,
                raw_path: &path,
                raw_query: "",
                headers: &signed,
            },
            &access_key,
            &secret,
            &region,
        )
        .map_err(|error| error.to_string())?;
        headers.push(("authorization".to_owned(), authorization));
        let url = format!("{}{path}", endpoint.trim_end_matches('/'));
        match self.client.send(method, &url, &headers, body).await? {
            (status, _) if (200..300).contains(&status) => Ok(()),
            (status, answer) => Err(format!("the destination answered {status}: {answer}")),
        }
    }

    /// The Role's user and the secret of its first live full key. A Role is an ARN whose last segment names the user
    /// (`arn:aws:iam::<account>:role/<user>`).
    async fn credential(&self, role: &str) -> std::result::Result<(String, SecretKey), String> {
        let name = role.rsplit('/').next().unwrap_or(role);
        let user = UserName::new(name).ok_or_else(|| format!("the Role names no user: {role}"))?;
        let users = self.storage.users();
        let key = users
            .keys_of(&user)
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|key| !key.disabled && key.scope.is_none())
            .ok_or_else(|| format!("user {name} has no live access key"))?;
        let resolved = users
            .resolve(&key.access_key_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("the access key of user {name} no longer resolves"))?;
        Ok((key.access_key_id, resolved.secret))
    }
}

/// The host (and port) part of `endpoint`, `https://host:port`.
fn host_of(endpoint: &str) -> Option<&str> {
    let rest = endpoint
        .split_once("://")
        .map_or(endpoint, |(_, rest)| rest);
    rest.split('/').next().filter(|host| !host.is_empty())
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
        })
}

/// Where a body stream stands: the pieces left, and the piece open now with its next block and block count.
struct Blocks {
    objects: ObjectService,
    pieces: std::vec::IntoIter<([u8; 16], u64)>,
    open: Option<(ObjectReader, u64, u64)>,
}

/// The bytes of `pieces` (data id, size), block by block, as a request body; an unreadable block ends it.
fn blocks(
    objects: ObjectService,
    pieces: Vec<([u8; 16], u64)>,
) -> impl Stream<Item = std::result::Result<Bytes, io::Error>> + Send + 'static {
    let start = Blocks {
        objects,
        pieces: pieces.into_iter(),
        open: None,
    };
    futures_util::stream::unfold(start, |mut at| async move {
        loop {
            if let Some((reader, index, count)) = at.open.take()
                && index < count
            {
                let block = reader
                    .read_block(index)
                    .await
                    .map(Bytes::from)
                    .map_err(|error| io::Error::other(error.to_string()));
                if block.is_err() {
                    at.pieces = Vec::new().into_iter();
                } else {
                    at.open = Some((reader, index.saturating_add(1), count));
                }
                return Some((block, at));
            }
            let (id, size) = at.pieces.next()?;
            match at.objects.open(id, size).await {
                Ok(reader) => {
                    let count = size.div_ceil(u64::from(reader.block_size()).max(1));
                    at.open = Some((reader, 0, count));
                }
                Err(error) => {
                    at.pieces = Vec::new().into_iter();
                    return Some((Err(io::Error::other(error.to_string())), at));
                }
            }
        }
    })
}

impl Daemon for Replicator {
    fn name(&self) -> &'static str {
        "replicator"
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(REPLICATION_POLL_SECS)
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(DAEMON_RUN_TIMEOUT_SECS)
    }

    fn run_once(&self) -> DaemonFuture<'_> {
        Box::pin(async move {
            let replication = self.storage.replication();
            let tasks = replication
                .next(REPLICATION_IN_FLIGHT)
                .await
                .map_err(failed)?;
            let (mut sent, mut retried) = (0_u64, 0_u64);
            for task in &tasks {
                match self.send(task).await {
                    Ok(()) => {
                        replication.completed(task).await.map_err(failed)?;
                        sent = sent.saturating_add(1);
                    }
                    Err(reason) => {
                        tracing::warn!(
                            bucket = %task.bucket,
                            key = %task.key,
                            version_id = %task.version_id,
                            deliveries = task.deliveries,
                            %reason,
                            "a version could not be replicated"
                        );
                        replication.failed(task).await.map_err(failed)?;
                        retried = retried.saturating_add(1);
                    }
                }
            }
            Ok(Report {
                message: format!("replicated {sent}, to retry {retried}"),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::host_of;

    #[test]
    fn the_signed_host_is_the_endpoints_host_and_port() {
        assert_eq!(
            host_of("https://s3.eu.example.test"),
            Some("s3.eu.example.test")
        );
        assert_eq!(host_of("http://127.0.0.1:9100"), Some("127.0.0.1:9100"));
        assert_eq!(host_of("http://"), None);
    }
}
