//! Reading and replacing one version's tags. The version is named by id or is the key's current one; the current
//! version's tags are also on its `objects` record, and both change in one transaction fenced on the version still
//! being current — a write of the key that commits first makes it refuse, and the change is planned again.

use std::collections::BTreeMap;

use tessari_s3_infrastructure::tessaridb::Value;
use tessari_s3_types::{BucketName, ObjectKey};

use super::super::attempts::Attempts;
use super::super::model_lock::LockOutcome;
use super::super::model_tags::TagsOutcome;
use super::super::version_id::NULL;
use super::{TessariObjects, id, is_lost_or_refused};
use crate::answers::first_record;
use crate::{Error, Result};

/// What addressing a version found when there is none to address.
fn absent(outcome: LockOutcome) -> TagsOutcome {
    match outcome {
        LockOutcome::NoSuchKey => TagsOutcome::NoSuchKey,
        LockOutcome::Malformed => TagsOutcome::Malformed,
        _ => TagsOutcome::NoSuchVersion,
    }
}

fn stored(tags: Option<&Value>) -> Result<BTreeMap<String, String>> {
    match tags {
        None | Some(Value::None) => Ok(BTreeMap::new()),
        Some(Value::Object(map)) => map
            .iter()
            .map(|(key, value)| match value {
                Value::String(text) => Ok((key.clone(), text.clone())),
                _ => Err(malformed()),
            })
            .collect(),
        Some(_) => Err(malformed()),
    }
}

const fn malformed() -> Error {
    Error::Malformed {
        record: "versions",
        reason: "tags",
    }
}

impl TessariObjects {
    /// The tags of the version `version` names, or of the key's current one.
    pub(crate) async fn version_tags(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        version: Option<&str>,
    ) -> Result<TagsOutcome> {
        Ok(match self.addressed(bucket, key, version).await? {
            Err(outcome) => absent(outcome),
            Ok(found) if found.marker => TagsOutcome::Marker,
            Ok(found) => TagsOutcome::Version {
                tags: stored(found.tags.as_ref())?,
                version_id: found.version_id,
            },
        })
    }

    /// Replaces the version's tags with `tags` (none removes them).
    pub(crate) async fn set_version_tags(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        version: Option<&str>,
        tags: &BTreeMap<String, String>,
    ) -> Result<TagsOutcome> {
        let value = if tags.is_empty() {
            Value::None
        } else {
            Value::Object(
                tags.iter()
                    .map(|(key, value)| (key.clone(), Value::String(value.clone())))
                    .collect(),
            )
        };
        let mut attempts = Attempts::new();
        while attempts.next().await {
            let found = match self.addressed(bucket, key, version).await? {
                Err(outcome) => return Ok(absent(outcome)),
                Ok(found) if found.marker => return Ok(TagsOutcome::Marker),
                Ok(found) => found,
            };
            let current = self.current_version(bucket, key).await?;
            let mut script = String::from(
                "BEGIN; UPDATE versions:$target SET tags = $tags WHERE version_id = $vid; ",
            );
            if current.as_deref() == Some(found.version_id.as_str()) {
                script.push_str(
                    "UPDATE objects:$id SET tags = $tags WHERE (version_id ?? $null) = $vid; ",
                );
            }
            script.push_str("COMMIT;");
            let parameters = vec![
                ("target".to_owned(), Value::Bytes(found.target.clone())),
                ("vid".to_owned(), Value::String(found.version_id.clone())),
                ("tags".to_owned(), value.clone()),
                ("id".to_owned(), id(bucket, key)),
                ("null".to_owned(), Value::String(NULL.to_owned())),
            ];
            match self.pool.run(&script, parameters).await {
                Ok(_) => {
                    return Ok(TagsOutcome::Version {
                        version_id: found.version_id,
                        tags: tags.clone(),
                    });
                }
                // The version stopped being current, or another write of the key won: address it again.
                Err(error) if is_lost_or_refused(&error) => {}
                Err(error) => return Err(error.into()),
            }
        }
        Err(Error::Contended)
    }

    /// The key's current version id (`null` for a never-versioned object); none when the key has no object.
    async fn current_version(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
    ) -> Result<Option<String>> {
        let answers = self
            .pool
            .run(
                "SELECT version_id FROM ONLY objects:$id;",
                vec![("id".to_owned(), id(bucket, key))],
            )
            .await?;
        Ok(match first_record(answers.into_iter().next())? {
            None => None,
            Some(Value::Object(fields)) => Some(match fields.get("version_id") {
                Some(Value::String(version_id)) => version_id.clone(),
                _ => NULL.to_owned(),
            }),
            Some(_) => return Err(malformed()),
        })
    }
}
