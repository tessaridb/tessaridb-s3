//! Delivery bookkeeping for one destination: hand out the next events, keep the ones its bucket's CURRENT rules send
//! to it, settle the rest at once, and settle or retry the delivered ones by their outcome.

use std::collections::BTreeMap;

use tessari_s3_constants::{NOTIFY_RETRY_MAX_SECS, NOTIFY_RETRY_STEP_SECS};
use tessari_s3_core::notification::{Rule, matches};
use tessari_s3_infrastructure::tessaridb::{Number, Value};
use tessari_s3_types::{BucketName, Timestamp};

use super::entity::MessageEntity;
use super::model::{Due, Event};
use super::repository::NotificationRepository;
use super::tessaridb::TessariNotifications;
use crate::buckets::BucketService;
use crate::{Error, Result};

/// Notification delivery operations.
#[derive(Clone)]
pub struct NotificationService {
    repository: TessariNotifications,
    buckets: BucketService,
}

const fn malformed(reason: &'static str) -> Error {
    Error::Malformed {
        record: "object_events",
        reason,
    }
}

fn event(message: MessageEntity) -> Result<Event> {
    let Value::Object(fields) = message.value else {
        return Err(malformed("a message is not an object"));
    };
    let text = |field: &'static str| match fields.get(field) {
        Some(Value::String(text)) => Ok(Some(text.clone())),
        None | Some(Value::None) => Ok(None),
        Some(_) => Err(malformed(field)),
    };
    let required = |field: &'static str| text(field)?.ok_or_else(|| malformed(field));
    let size = match fields.get("size") {
        Some(Value::Number(Number::Integer(size))) => {
            Some(u64::try_from(*size).map_err(|_| malformed("size"))?)
        }
        None | Some(Value::None) => None,
        Some(_) => return Err(malformed("size")),
    };
    let at = match fields.get("at") {
        Some(Value::Datetime { seconds, nanos }) => Timestamp {
            seconds: *seconds,
            nanos: *nanos,
        },
        _ => return Err(malformed("at")),
    };
    Ok(Event {
        position: message.position,
        deliveries: message.deliveries,
        bucket: required("bucket_name")?,
        key: required("key")?,
        name: required("event")?,
        version_id: text("version_id")?,
        size,
        etag: text("etag")?,
        at,
    })
}

/// Seconds before a delivery handed out `deliveries` times is tried again: doubling from the step, capped.
fn retry_after(deliveries: u64) -> u64 {
    let doublings = u32::try_from(deliveries.saturating_sub(1))
        .unwrap_or(u32::MAX)
        .min(16);
    NOTIFY_RETRY_STEP_SECS
        .saturating_mul(1_u64 << doublings)
        .min(NOTIFY_RETRY_MAX_SECS)
}

impl NotificationService {
    /// Delivery bookkeeping over `repository`, matching through `buckets`.
    #[must_use]
    pub const fn new(repository: TessariNotifications, buckets: BucketService) -> Self {
        Self {
            repository,
            buckets,
        }
    }

    /// Declares each target's group when it is missing.
    ///
    /// # Errors
    /// The metadata store's refusal or outage, or a target id that is not a name.
    pub async fn define<'a>(&self, targets: impl IntoIterator<Item = &'a str>) -> Result<()> {
        for target in targets {
            self.repository.define(target).await?;
        }
        Ok(())
    }

    /// The next events for `target` (at most `limit` handed out): those a rule of their bucket's current
    /// configuration sends to `target`. The others are settled here; one that cannot be read is handed back, so the
    /// group's dead letter keeps it rather than it vanishing.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn next(&self, target: &str, limit: usize) -> Result<Vec<Due>> {
        let mut rules_of: BTreeMap<String, Vec<Rule>> = BTreeMap::new();
        let (mut due, mut unmatched, mut unreadable) = (Vec::new(), Vec::new(), Vec::new());
        for message in self.repository.read(target, limit).await? {
            let (position, deliveries) = (message.position, message.deliveries);
            let event = match event(message) {
                Ok(event) => event,
                Err(error) => {
                    tracing::warn!(target, position, %error, "an event message cannot be read");
                    unreadable.push((position, deliveries));
                    continue;
                }
            };
            if !rules_of.contains_key(&event.bucket) {
                let rules = match BucketName::new(&event.bucket) {
                    Ok(name) => self
                        .buckets
                        .get(&name)
                        .await?
                        .and_then(|bucket| bucket.notification)
                        .unwrap_or_default(),
                    Err(_) => Vec::new(),
                };
                rules_of.insert(event.bucket.clone(), rules);
            }
            let rule = rules_of.get(&event.bucket).and_then(|rules| {
                rules
                    .iter()
                    .find(|rule| rule.target == target && matches(rule, &event.name, &event.key))
            });
            match rule {
                Some(rule) => due.push(Due {
                    configuration: rule.id.clone(),
                    event,
                }),
                None => unmatched.push(event.position),
            }
        }
        self.repository.ack(target, &unmatched).await?;
        for (position, deliveries) in unreadable {
            self.repository
                .nack(target, &[position], retry_after(deliveries))
                .await?;
        }
        Ok(due)
    }

    /// Appends `body` (an S3 event notification, JSON) for the event at `position` to the `s3_notifications` topic
    /// under the name `n-<position>`; a second append for one event leaves the first, so the topic holds each once.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn publish(&self, position: u64, body: &str) -> Result<()> {
        self.repository.publish(position, body).await
    }

    /// Settles the events at `positions`, delivered.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn delivered(&self, target: &str, positions: &[u64]) -> Result<()> {
        self.repository.ack(target, positions).await
    }

    /// Hands `event` out again after a wait that grows with its hand-outs; past the group's limit it goes to the dead
    /// letter instead.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn failed(&self, target: &str, event: &Event) -> Result<()> {
        self.repository
            .nack(target, &[event.position], retry_after(event.deliveries))
            .await
    }
}
