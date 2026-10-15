//! Replication rules: a task is parsed from the message the store's event wrote; a sent version is stamped
//! `COMPLETED` and its task settled; a version not sent is tried again after a doubling delay, and on its last
//! hand-out is stamped `FAILED` before the group gives it up to `replication_failed`.

use tessari_s3_constants::{
    REPLICATION_DELIVERIES, REPLICATION_RETRY_MAX_SECS, REPLICATION_RETRY_STEP_SECS,
};
use tessari_s3_infrastructure::tessaridb::{Number, Value};

use super::model::{Progress, Task};
use super::repository::ReplicationRepository;
use super::tessaridb::TessariReplication;
use crate::topic_groups::{self, MessageEntity};
use crate::{Error, Result};

const fn malformed(reason: &'static str) -> Error {
    Error::Malformed {
        record: "replication_tasks",
        reason,
    }
}

/// The task a handed-out message holds.
fn task(message: MessageEntity) -> Result<Task> {
    let Value::Object(fields) = message.value else {
        return Err(malformed("a message is not an object"));
    };
    let text = |field: &'static str| match fields.get(field) {
        Some(Value::String(text)) => Ok(text.clone()),
        _ => Err(malformed(field)),
    };
    let seq = match fields.get("seq") {
        Some(Value::Number(Number::Integer(seq))) => {
            u64::try_from(*seq).map_err(|_| malformed("seq"))?
        }
        _ => return Err(malformed("seq")),
    };
    let marker = match fields.get("marker") {
        Some(Value::Bool(marker)) => *marker,
        _ => return Err(malformed("marker")),
    };
    Ok(Task {
        position: message.position,
        deliveries: message.deliveries,
        bucket: text("bucket_name")?,
        key: text("key")?,
        version_id: text("version_id")?,
        seq,
        marker,
        destination: text("destination")?,
    })
}

/// Whether a task handed out `deliveries` times is on its last hand-out.
fn last_try(deliveries: u64) -> bool {
    deliveries >= u64::from(REPLICATION_DELIVERIES)
}

/// Replication of this region's versions.
#[derive(Clone)]
pub struct ReplicationService {
    repository: TessariReplication,
}

impl ReplicationService {
    /// The service over `repository`.
    #[must_use]
    pub(crate) const fn new(repository: TessariReplication) -> Self {
        Self { repository }
    }

    /// Declares the replicators' group when it is missing.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn define(&self) -> Result<()> {
        self.repository.define().await
    }

    /// Hands out up to `limit` tasks.
    ///
    /// # Errors
    /// The metadata store's refusal or outage, or a message the event did not write.
    pub async fn next(&self, limit: usize) -> Result<Vec<Task>> {
        self.repository
            .read(limit)
            .await?
            .into_iter()
            .map(task)
            .collect()
    }

    /// `task`'s version reached its destination: stamped `COMPLETED`, the task settled.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn completed(&self, task: &Task) -> Result<()> {
        self.repository
            .stamp(
                &task.bucket,
                &task.key,
                task.seq,
                &task.version_id,
                "COMPLETED",
            )
            .await?;
        self.repository.ack(&[task.position]).await
    }

    /// `task`'s version did not reach its destination: tried again later, or — on its last hand-out — stamped `FAILED`
    /// and given up to `replication_failed`.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn failed(&self, task: &Task) -> Result<()> {
        if last_try(task.deliveries) {
            self.repository
                .stamp(
                    &task.bucket,
                    &task.key,
                    task.seq,
                    &task.version_id,
                    "FAILED",
                )
                .await?;
        }
        let delay = topic_groups::retry_after(
            task.deliveries,
            REPLICATION_RETRY_STEP_SECS,
            REPLICATION_RETRY_MAX_SECS,
        );
        self.repository.nack(&[task.position], delay).await
    }

    /// The replicators' backlog, in flight and given-up counts.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn progress(&self) -> Result<Progress> {
        self.repository.progress().await
    }

    /// Records that `region`'s S3 surface is reached at `endpoint`.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn advertise(&self, region: &str, endpoint: &str) -> Result<()> {
        self.repository.advertise(region, endpoint).await
    }

    /// The URL `region`'s S3 surface is reached at, if any of its nodes advertised one.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn endpoint(&self, region: &str) -> Result<Option<String>> {
        self.repository.endpoint(region).await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use tessari_s3_constants::REPLICATION_DELIVERIES;
    use tessari_s3_infrastructure::tessaridb::{Number, Value};

    use super::{last_try, task};
    use crate::topic_groups::MessageEntity;

    fn message(fields: &[(&str, Value)]) -> MessageEntity {
        let fields: BTreeMap<String, Value> = fields
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect();
        MessageEntity {
            position: 7,
            deliveries: 2,
            value: Value::Object(fields),
        }
    }

    fn written() -> Vec<(&'static str, Value)> {
        vec![
            ("bucket_name", Value::String("origin".to_owned())),
            ("key", Value::String("logs/a".to_owned())),
            (
                "version_id",
                Value::String("0000000000000003000000000000abcd".to_owned()),
            ),
            ("seq", Value::Number(Number::Integer(3))),
            ("marker", Value::Bool(false)),
            ("destination", Value::String("copies".to_owned())),
            (
                "at",
                Value::Datetime {
                    seconds: 1,
                    nanos: 0,
                },
            ),
        ]
    }

    #[test]
    fn a_task_reads_back_what_the_event_wrote() {
        let parsed = task(message(&written())).expect("a task");
        assert_eq!(
            (
                parsed.position,
                parsed.deliveries,
                parsed.bucket.as_str(),
                parsed.key.as_str(),
                parsed.version_id.as_str(),
                parsed.seq,
                parsed.marker,
                parsed.destination.as_str()
            ),
            (
                7,
                2,
                "origin",
                "logs/a",
                "0000000000000003000000000000abcd",
                3,
                false,
                "copies"
            )
        );
    }

    #[test]
    fn a_task_missing_a_field_or_holding_the_wrong_kind_is_refused() {
        for (field, wrong) in [
            ("destination", Value::None),
            ("seq", Value::String("3".to_owned())),
            ("marker", Value::String("false".to_owned())),
            ("version_id", Value::Number(Number::Integer(3))),
        ] {
            let mut fields = written();
            fields.retain(|(name, _)| *name != field);
            fields.push((field, wrong));
            assert!(task(message(&fields)).is_err(), "{field}");
        }
    }

    #[test]
    fn only_the_last_hand_out_gives_a_version_up() {
        let last = u64::from(REPLICATION_DELIVERIES);
        assert!(last_try(last));
        assert!(!last_try(last - 1));
        assert!(!last_try(1));
    }
}
