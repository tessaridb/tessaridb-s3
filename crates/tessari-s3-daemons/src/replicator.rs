//! Replication (S3 ref 11 M4): the tasks this region's group hands out are versions to send to their destination
//! bucket. Each is sent through the destination region's S3 surface as a replica — PutObject, or DeleteObject for a
//! delete marker, carrying the source version id — signed as the user the source's configuration names as its Role,
//! so the destination decides on it as on any request. A sent version is stamped `COMPLETED`; one that cannot be sent
//! waits and is tried again, and is stamped `FAILED` when the group gives it up.

use std::time::Duration;

use tessari_s3_constants::{DAEMON_RUN_TIMEOUT_SECS, REPLICATION_IN_FLIGHT, REPLICATION_POLL_SECS};
use tessari_s3_core::auth::{SignedRequest, amz_date, sign_header, uri_encode};
use tessari_s3_core::authz::UserName;
use tessari_s3_core::objects::checksum::{ChecksumAlgorithm, Declared};
use tessari_s3_infrastructure::ReplicaClient;
use tessari_s3_storage::Storage;
use tessari_s3_storage::encryption::{Sealing, WrappedKey};
use tessari_s3_storage::objects::{Content, VersionRead};
use tessari_s3_storage::replication::Task;
use tessari_s3_types::{BucketName, ObjectKey, SecretKey};

use crate::Error;
use crate::daemon::{Daemon, DaemonFuture, Report};

#[path = "replicator_body.rs"]
mod body;
use body::blocks;

/// The header a replica carries: the version id its source gave it.
const REPLICA_VERSION: &str = "x-tessaridb-replica-version-id";

/// The headers that ask the destination to seal a replica as its source is sealed: under the server's own key, or
/// under the key of the same name.
fn sealing_headers(sealing: &Sealing) -> Vec<(String, String)> {
    let mode = "x-amz-server-side-encryption".to_owned();
    match sealing {
        Sealing::Server => vec![(mode, "AES256".to_owned())],
        Sealing::Named(name) => vec![
            (mode, "aws:kms".to_owned()),
            (
                "x-amz-server-side-encryption-aws-kms-key-id".to_owned(),
                name.to_string(),
            ),
        ],
    }
}

/// What sending a version came to.
enum Sent {
    /// The destination holds it, or nothing is left to send.
    Sent,
    /// No attempt could ever send it, for the reason given.
    Unsendable(&'static str),
}

/// One stored piece of a version: its data id, its size, and the wrapped key it is sealed under, if any.
type Piece = ([u8; 16], u64, Option<WrappedKey>);

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

    /// Sends `task`'s version; `Ok(Sent)` once the destination holds it (or the version is gone and there is nothing
    /// to send), `Ok(Unsendable)` when no attempt ever could, `Err` with the reason otherwise.
    async fn send(&self, task: &Task) -> std::result::Result<Sent, String> {
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
                VersionRead::Object(object) => *object,
                // Removed by its id since it was queued: nothing is left to send.
                VersionRead::Absent => return Ok(Sent::Sent),
                other => return Err(format!("the version cannot be read: {other:?}")),
            };
            // A replica would need the customer's key to be opened and sealed again, and that key is never kept.
            if object.customer_seal().is_some() {
                return Ok(Sent::Unsendable(
                    "the version is sealed under a customer-provided key, which is never kept",
                ));
            }
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
            let pieces: Vec<Piece> = match &object.content {
                Content::Data(id) => vec![(*id, object.size, object.encryption.clone())],
                Content::Parts(multipart) => multipart
                    .parts
                    .iter()
                    .map(|part| (part.data, part.size, part.encryption.clone()))
                    .collect(),
            };
            // A sealed source is sealed at the destination too, so a replica never lands in clear; a key id this
            // build does not know is not sent plain.
            if let Some((_, _, Some(wrapped))) =
                pieces.iter().find(|(_, _, sealed)| sealed.is_some())
            {
                let sealing = Sealing::of(wrapped)
                    .ok_or("the source names a key this build does not know")?;
                headers.extend(sealing_headers(&sealing));
            }
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
            (status, _) if (200..300).contains(&status) => Ok(Sent::Sent),
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
                    Ok(Sent::Sent) => {
                        replication.completed(task).await.map_err(failed)?;
                        sent = sent.saturating_add(1);
                    }
                    Ok(Sent::Unsendable(reason)) => {
                        tracing::warn!(
                            bucket = %task.bucket,
                            key = %task.key,
                            version_id = %task.version_id,
                            reason,
                            "a version cannot be replicated"
                        );
                        replication.unsendable(task).await.map_err(failed)?;
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
