//! The notification worker for one webhook destination: delivers the events its group hands out (see
//! `NotificationService::next`) as S3 event records, a few at once, settling each by its outcome — at least once,
//! never before the change committed, and a destination that keeps failing has its events dead-lettered.

use std::time::Duration;

use futures_util::StreamExt as _;
use serde::Serialize;
use tessari_s3_constants::{
    DAEMON_RUN_TIMEOUT_SECS, NOTIFY_BATCH, NOTIFY_CONCURRENCY, NOTIFY_POLL_SECS,
};
use tessari_s3_core::auth::uri_encode;
use tessari_s3_infrastructure::WebhookClient;
use tessari_s3_storage::notifications::{Due, NotificationService};

use crate::Error;
use crate::daemon::{Daemon, DaemonFuture, Report};

/// Delivers one destination's events to its webhook.
pub struct NotifyWorker {
    notifications: NotificationService,
    target: String,
    url: String,
    region: String,
    client: WebhookClient,
}

impl NotifyWorker {
    /// A worker delivering `target`'s events to `url`, naming `region` in each record.
    #[must_use]
    pub const fn new(
        notifications: NotificationService,
        target: String,
        url: String,
        region: String,
        client: WebhookClient,
    ) -> Self {
        Self {
            notifications,
            target,
            url,
            region,
            client,
        }
    }
}

/// The body of one delivery: an S3 event notification with one record.
#[derive(Serialize)]
struct Body<'a> {
    #[serde(rename = "Records")]
    records: [Record<'a>; 1],
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Record<'a> {
    event_version: &'static str,
    event_source: &'static str,
    aws_region: &'a str,
    event_time: String,
    event_name: &'a str,
    s3: S3<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct S3<'a> {
    s3_schema_version: &'static str,
    configuration_id: &'a str,
    bucket: Bucket<'a>,
    object: Object<'a>,
}

#[derive(Serialize)]
struct Bucket<'a> {
    name: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Object<'a> {
    key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    e_tag: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    version_id: Option<&'a str>,
    sequencer: String,
}

/// The key as S3 event records carry it: URL-encoded, a space as `+`.
fn event_key(key: &str) -> String {
    let mut encoded = String::with_capacity(key.len());
    uri_encode(key.as_bytes(), true, &mut encoded);
    encoded.replace("%20", "+")
}

/// The JSON body delivering `due`.
fn body(due: &Due, region: &str) -> Result<bytes::Bytes, serde_json::Error> {
    let event = &due.event;
    let record = Record {
        event_version: "2.1",
        event_source: "tessaridb:s3",
        aws_region: region,
        event_time: event.at.iso8601_millis(),
        event_name: event.name.strip_prefix("s3:").unwrap_or(&event.name),
        s3: S3 {
            s3_schema_version: "1.0",
            configuration_id: &due.configuration,
            bucket: Bucket {
                name: &event.bucket,
            },
            object: Object {
                key: event_key(&event.key),
                size: event.size,
                e_tag: event.etag.as_deref(),
                version_id: event.version_id.as_deref(),
                sequencer: format!("{:016X}", event.position),
            },
        },
    };
    serde_json::to_vec(&Body { records: [record] }).map(bytes::Bytes::from)
}

fn failed(message: impl ToString) -> Error {
    Error::Run {
        daemon: "notify",
        message: message.to_string(),
    }
}

impl Daemon for NotifyWorker {
    fn name(&self) -> &'static str {
        "notify"
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(NOTIFY_POLL_SECS)
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(DAEMON_RUN_TIMEOUT_SECS)
    }

    fn run_once(&self) -> DaemonFuture<'_> {
        Box::pin(async move {
            let (mut sent, mut retried) = (0_u64, 0_u64);
            // Batches until one comes back with nothing to deliver; the runner's timeout bounds the run.
            loop {
                let due = self
                    .notifications
                    .next(&self.target, NOTIFY_BATCH)
                    .await
                    .map_err(failed)?;
                if due.is_empty() {
                    break;
                }
                let outcomes: Vec<(Due, bool)> = futures_util::stream::iter(due)
                    .map(|due| async move {
                        let delivered = match body(&due, &self.region) {
                            Ok(body) => self.client.post(&self.url, body).await,
                            Err(error) => {
                                tracing::error!(%error, "a notification could not be encoded");
                                false
                            }
                        };
                        (due, delivered)
                    })
                    .buffer_unordered(NOTIFY_CONCURRENCY)
                    .collect()
                    .await;
                let delivered: Vec<u64> = outcomes
                    .iter()
                    .filter(|(_, delivered)| *delivered)
                    .map(|(due, _)| due.event.position)
                    .collect();
                self.notifications
                    .delivered(&self.target, &delivered)
                    .await
                    .map_err(failed)?;
                sent = sent.saturating_add(u64::try_from(delivered.len()).unwrap_or(u64::MAX));
                for (due, _) in outcomes.iter().filter(|(_, delivered)| !*delivered) {
                    self.notifications
                        .failed(&self.target, &due.event)
                        .await
                        .map_err(failed)?;
                    retried = retried.saturating_add(1);
                }
            }
            Ok(Report {
                message: format!(
                    "target {}: delivered {sent}, to retry {retried}",
                    self.target
                ),
            })
        })
    }
}

#[cfg(test)]
#[path = "notify_tests.rs"]
mod tests;
