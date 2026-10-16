//! The statements a replica needs beyond an ordinary write: the id stored at a key's sequence, and a version added
//! behind the current one without touching the key's `objects` record.

use tessari_s3_infrastructure::tessaridb::Value;
use tessari_s3_types::{BucketName, ObjectKey};

use super::super::entity::record;
use super::super::model::{Content, NewObject};
use super::super::model_lock::Lock;
use super::super::repository::VersionPlan;
use super::super::version_id::{key_prefix, record_id};
use super::{is_lost_or_refused, versions};
use crate::answers::first_record;
use crate::{Error, Result};

impl super::TessariObjects {
    /// The version id stored at sequence `seq` of `bucket/key`, if a version is stored there.
    pub(crate) async fn version_id_at(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        seq: u64,
    ) -> Result<Option<String>> {
        let at = record_id(&key_prefix(bucket.as_str(), key.as_str()), seq);
        let answers = self
            .pool
            .run(
                "SELECT version_id FROM ONLY versions:$at;",
                vec![("at".to_owned(), Value::Bytes(at))],
            )
            .await?;
        match first_record(answers.into_iter().next())? {
            None => Ok(None),
            Some(Value::Object(fields)) => match fields.get("version_id") {
                Some(Value::String(id)) => Ok(Some(id.clone())),
                _ => Err(Error::Malformed {
                    record: "versions",
                    reason: "version_id",
                }),
            },
            Some(_) => Err(Error::Malformed {
                record: "versions",
                reason: "not an object",
            }),
        }
    }

    /// Adds the version `plan` creates — holding `object`, or a delete marker — behind the key's current version, as
    /// ONE transaction; `false` when its sequence was taken meanwhile or its data file was reclaimed.
    pub(crate) async fn add_version(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        incarnation: [u8; 16],
        object: Option<&NewObject>,
        plan: &VersionPlan,
    ) -> Result<bool> {
        let changes = versions::statements(
            bucket,
            key,
            incarnation,
            plan,
            object.is_some(),
            &Lock::default(),
        )?;
        let mut parameters = changes.parameters;
        // The data commits with the removal of its pending `reclaims` entry, as an ordinary write's does.
        let reclaim = match object {
            Some(object) => {
                let Content::Data(data) = &object.content else {
                    return Err(Error::Malformed {
                        record: "object",
                        reason: "a replica is written whole, never from parts",
                    });
                };
                parameters.push((
                    "object".to_owned(),
                    record(bucket.as_str(), key.as_str(), incarnation, object)?,
                ));
                parameters.push(("data".to_owned(), Value::Uuid(*data)));
                "UPDATE reclaims:$data SET data = $data WHERE reclaiming = NONE; DELETE reclaims:$data; "
            }
            None => "",
        };
        let script = format!(
            "BEGIN; LET $now = time::now(); {reclaim}{}{}",
            changes.script,
            versions::commit(plan.local)
        );
        match self.pool.run(&script, parameters).await {
            Ok(_) => Ok(true),
            Err(error) if is_lost_or_refused(&error) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }
}
