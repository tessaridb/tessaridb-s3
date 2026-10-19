//! Reading and changing one version's Object Lock (S3 ref 08 M5): its retention and its legal hold. The version is
//! named by id or is the key's current one. Whether a retention still keeps the version is judged by the metadata
//! store's clock, and the store's `locked_change` event refuses any update that would release it (ADR-0016).

use tessari_s3_core::object_lock::Mode;
use tessari_s3_infrastructure::tessaridb::Value;
use tessari_s3_types::{BucketName, ObjectKey, Timestamp};

use super::super::entity::stored_lock;
use super::super::model_lock::{LockOutcome, VersionLock};
use super::super::version_id::{NULL, Naming, key_prefix, parse, record_id};
use super::version_delete::null_seq;
use super::{TessariObjects, id, is_lock_refusal};
use crate::answers::first_record;
use crate::{Error, Result};

const fn malformed(reason: &'static str) -> Error {
    Error::Malformed {
        record: "versions",
        reason,
    }
}

/// The record a request addresses, as read.
pub(super) struct Addressed {
    pub(super) target: Vec<u8>,
    pub(super) version_id: String,
    pub(super) marker: bool,
    /// Its tags, as stored.
    pub(super) tags: Option<Value>,
    lock: VersionLock,
    /// Whether its retention has not passed, by the store's clock.
    retained: bool,
}

impl TessariObjects {
    /// The version `version` names (the current one when `None`); `Err(outcome)` when there is none to address.
    pub(super) async fn addressed(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        version: Option<&str>,
    ) -> Result<std::result::Result<Addressed, LockOutcome>> {
        let prefix = key_prefix(bucket.as_str(), key.as_str());
        let version_id = match version {
            Some(version) => version.to_owned(),
            None => {
                let answers = self
                    .pool
                    .run(
                        "SELECT version_id FROM ONLY objects:$id;",
                        vec![("id".to_owned(), id(bucket, key))],
                    )
                    .await?;
                match first_record(answers.into_iter().next())? {
                    None => return Ok(Err(LockOutcome::NoSuchKey)),
                    Some(Value::Object(fields)) => match fields.get("version_id") {
                        Some(Value::String(version_id)) => version_id.clone(),
                        _ => NULL.to_owned(),
                    },
                    Some(_) => return Err(malformed("an object record")),
                }
            }
        };
        let seq = match parse(&version_id) {
            None => return Ok(Err(LockOutcome::Malformed)),
            Some(Naming::Seq(seq)) => seq,
            Some(Naming::Null) => match null_seq(&self.pool, &prefix).await? {
                Some(seq) => seq,
                None => return Ok(Err(LockOutcome::NoSuchVersion)),
            },
        };
        let target = record_id(&prefix, seq);
        let answers = self
            .pool
            .run(
                "SELECT version_id, marker, tags, lock_mode, retain_until, legal_hold, \
                 (retain_until != NONE AND retain_until > time::now()) AS retained FROM ONLY versions:$target;",
                vec![("target".to_owned(), Value::Bytes(target.clone()))],
            )
            .await?;
        let Some(Value::Object(fields)) = first_record(answers.into_iter().next())? else {
            return Ok(Err(LockOutcome::NoSuchVersion));
        };
        if fields.get("version_id") != Some(&Value::String(version_id.clone())) {
            return Ok(Err(LockOutcome::NoSuchVersion));
        }
        Ok(Ok(Addressed {
            target,
            marker: fields.get("marker") == Some(&Value::Bool(true)),
            tags: fields.get("tags").cloned(),
            lock: stored_lock(&fields)?,
            retained: fields.get("retained") == Some(&Value::Bool(true)),
            version_id,
        }))
    }

    /// The Object Lock of the version `version` names, or of the key's current one.
    pub(crate) async fn version_lock(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        version: Option<&str>,
    ) -> Result<LockOutcome> {
        Ok(match self.addressed(bucket, key, version).await? {
            Err(outcome) => outcome,
            Ok(found) if found.marker => LockOutcome::Marker,
            Ok(found) => LockOutcome::Version {
                version_id: found.version_id,
                lock: found.lock,
            },
        })
    }

    /// Sets the version's retention (`None` removes it). A change that would shorten or remove a retention still
    /// keeping it — COMPLIANCE always, GOVERNANCE unless `bypass` — is [`LockOutcome::Locked`].
    pub(crate) async fn set_retention(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        version: Option<&str>,
        retention: Option<(Mode, Timestamp)>,
        bypass: bool,
    ) -> Result<LockOutcome> {
        let found = match self.addressed(bucket, key, version).await? {
            Err(outcome) => return Ok(outcome),
            Ok(found) if found.marker => return Ok(LockOutcome::Marker),
            Ok(found) => found,
        };
        if let (true, Some((mode, until))) = (found.retained, found.lock.retention) {
            let releases = match retention {
                None => true,
                Some((new_mode, new_until)) => {
                    new_until < until || (mode == Mode::Compliance && new_mode != Mode::Compliance)
                }
            };
            if releases && (mode == Mode::Compliance || !bypass) {
                return Ok(LockOutcome::Locked);
            }
        }
        let mut parameters = vec![
            ("target".to_owned(), Value::Bytes(found.target.clone())),
            ("vid".to_owned(), Value::String(found.version_id.clone())),
        ];
        let assignment = match retention {
            Some((mode, until)) => {
                parameters.push(("mode".to_owned(), Value::String(mode.as_str().to_owned())));
                parameters.push((
                    "until".to_owned(),
                    Value::Datetime {
                        seconds: until.seconds,
                        nanos: until.nanos,
                    },
                ));
                "lock_mode = $mode, retain_until = $until"
            }
            None => "lock_mode = NONE, retain_until = NONE",
        };
        // The bypass flag is set and cleared inside this transaction, which is what the store's event reads.
        let script = if bypass {
            format!(
                "BEGIN; UPDATE versions:$target SET lock_bypass = true WHERE version_id = $vid; \
                 UPDATE versions:$target SET {assignment}, lock_bypass = NONE WHERE version_id = $vid; COMMIT;"
            )
        } else {
            format!("UPDATE versions:$target SET {assignment} WHERE version_id = $vid;")
        };
        match self.pool.run(&script, parameters).await {
            Ok(_) => {}
            // Only the store's `locked_change` event refuses this typed update: the retention began keeping the
            // version between the read and the write.
            Err(error) if is_lock_refusal(&error) => return Ok(LockOutcome::Locked),
            Err(error) => return Err(error.into()),
        }
        Ok(LockOutcome::Version {
            version_id: found.version_id,
            lock: VersionLock {
                retention,
                legal_hold: found.lock.legal_hold,
            },
        })
    }

    /// Turns the version's legal hold on or off.
    pub(crate) async fn set_legal_hold(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        version: Option<&str>,
        on: bool,
    ) -> Result<LockOutcome> {
        let found = match self.addressed(bucket, key, version).await? {
            Err(outcome) => return Ok(outcome),
            Ok(found) if found.marker => return Ok(LockOutcome::Marker),
            Ok(found) => found,
        };
        let parameters = vec![
            ("target".to_owned(), Value::Bytes(found.target.clone())),
            ("vid".to_owned(), Value::String(found.version_id.clone())),
            ("on".to_owned(), Value::Bool(on)),
        ];
        self.pool
            .run(
                "UPDATE versions:$target SET legal_hold = $on WHERE version_id = $vid;",
                parameters,
            )
            .await?;
        Ok(LockOutcome::Version {
            version_id: found.version_id,
            lock: VersionLock {
                retention: found.lock.retention,
                legal_hold: on,
            },
        })
    }
}
