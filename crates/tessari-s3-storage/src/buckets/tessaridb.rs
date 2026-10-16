//! Buckets in TessariDB: one `buckets` record per bucket, id = the bucket name; every value is bound.

use tessari_s3_core::authz::{SpaceName, Visible};
use tessari_s3_infrastructure::tessaridb::{
    Answer, MetaError, MetaPool, Number, RefusalClass, Value,
};
use tessari_s3_types::BucketName;

use super::entity::BucketEntity;
use super::model::LOCAL;
use super::repository::{BucketRepository, Inserted};
use crate::answers::first_record;
use crate::{Error, Result};

/// The TessariDB bucket repository.
#[derive(Clone)]
pub struct TessariBuckets {
    /// The global database, where buckets live.
    pool: MetaPool,
    /// This region's database: whether a bucket still holds objects, and its place in the lifecycle queue.
    regional: MetaPool,
}

impl TessariBuckets {
    /// A repository over the global `pool` and this region's `regional` handle.
    #[must_use]
    pub const fn new(pool: MetaPool, regional: MetaPool) -> Self {
        Self { pool, regional }
    }
}

fn name_param(name: &BucketName) -> Vec<(String, Value)> {
    vec![("name".to_owned(), Value::String(name.as_str().to_owned()))]
}

/// The one record or value a single-statement answer carries, or `None` when it carries none.
fn single(answers: Vec<Answer>) -> Result<Option<Value>> {
    match answers.into_iter().next() {
        Some(Answer::Records { records, .. }) => {
            Ok(records.into_iter().next().map(|(_, value)| value))
        }
        Some(Answer::Value {
            value: Value::None, ..
        })
        | Some(Answer::Done) => Ok(None),
        Some(Answer::Value { value, .. }) => Ok(Some(value)),
        _ => Err(Error::Malformed {
            record: "bucket",
            reason: "unexpected answer kind",
        }),
    }
}

impl BucketRepository for TessariBuckets {
    async fn insert(
        &self,
        name: &BucketName,
        region: &str,
        space: &SpaceName,
        creator: Option<&str>,
        object_lock: bool,
    ) -> Result<Inserted> {
        let mut parameters = name_param(name);
        parameters.push(("region".to_owned(), Value::String(region.to_owned())));
        parameters.push(("space".to_owned(), Value::String(space.as_str().to_owned())));
        let creator = creator.map_or(Value::None, |creator| Value::String(creator.to_owned()));
        parameters.push(("creator".to_owned(), creator));
        let script = if object_lock {
            "CREATE buckets:$name = { name: $name, created: time::now(), region: $region, \
             incarnation: rand::uuid(), space_name: $space, creator: $creator, versioning: 'Enabled', \
             object_lock: {} } RETURN AFTER;"
        } else {
            "CREATE buckets:$name = { name: $name, created: time::now(), region: $region, \
             incarnation: rand::uuid(), space_name: $space, creator: $creator } RETURN AFTER;"
        };
        match self.pool.run(script, parameters).await {
            Ok(answers) => match single(answers)? {
                Some(value) => Ok(Inserted::Created(Box::new(BucketEntity::from_value(
                    &value,
                )?))),
                None => Err(Error::Malformed {
                    record: "bucket",
                    reason: "CREATE answered no record",
                }),
            },
            Err(error) if error.is_class(RefusalClass::Conflict) => Ok(Inserted::Exists),
            Err(error) => Err(error.into()),
        }
    }

    async fn get(&self, name: &BucketName) -> Result<Option<BucketEntity>> {
        let answers = self
            .pool
            .run("SELECT * FROM ONLY buckets:$name;", name_param(name))
            .await?;
        single(answers)?
            .map(|value| BucketEntity::from_value(&value))
            .transpose()
    }

    async fn set_versioning(&self, name: &BucketName, state: &'static str) -> Result<()> {
        let mut parameters = name_param(name);
        parameters.push(("state".to_owned(), Value::String(state.to_owned())));
        self.pool
            .run("UPDATE buckets:$name SET versioning = $state;", parameters)
            .await?;
        Ok(())
    }

    async fn set_object_lock(&self, name: &BucketName, configuration: Value) -> Result<bool> {
        let mut parameters = name_param(name);
        parameters.push(("configuration".to_owned(), configuration));
        let answers = self
            .pool
            .run(
                "UPDATE buckets:$name SET object_lock = $configuration WHERE versioning = 'Enabled' RETURN AFTER;",
                parameters,
            )
            .await?;
        Ok(single(answers)?.is_some())
    }

    async fn set_encryption(&self, name: &BucketName, encryption: Option<Value>) -> Result<()> {
        let mut parameters = name_param(name);
        parameters.push(("encryption".to_owned(), encryption.unwrap_or(Value::None)));
        self.pool
            .run(
                "UPDATE buckets:$name SET encryption = $encryption;",
                parameters,
            )
            .await?;
        Ok(())
    }

    async fn set_policy(&self, name: &BucketName, policy: Option<String>) -> Result<()> {
        let mut parameters = name_param(name);
        parameters.push((
            "policy".to_owned(),
            policy.map_or(Value::None, Value::String),
        ));
        self.pool
            .run("UPDATE buckets:$name SET policy = $policy;", parameters)
            .await?;
        Ok(())
    }

    async fn set_tags(&self, name: &BucketName, tags: Option<Value>) -> Result<()> {
        let mut parameters = name_param(name);
        parameters.push(("tags".to_owned(), tags.unwrap_or(Value::None)));
        self.pool
            .run("UPDATE buckets:$name SET tags = $tags;", parameters)
            .await?;
        Ok(())
    }

    async fn set_replication(&self, name: &BucketName, configuration: Option<Value>) -> Result<()> {
        let mut parameters = name_param(name);
        let script = match configuration {
            Some(configuration) => {
                parameters.push(("configuration".to_owned(), configuration));
                "UPDATE buckets:$name SET replication = $configuration;"
            }
            None => "UPDATE buckets:$name SET replication = NONE;",
        };
        self.pool.run(script, parameters).await?;
        Ok(())
    }

    async fn set_lifecycle(&self, name: &BucketName, rules: Option<Value>) -> Result<()> {
        let mut parameters = name_param(name);
        // The rules are the bucket's, in the global database; the queue is the region's. No transaction writes both, so
        // they are written in the order a crash between them leaves harmless: a queued bucket without rules is a pass
        // that finds nothing to do, while rules without a queued bucket would never run.
        match rules {
            Some(rules) => {
                self.regional
                    .run(
                        "UPSERT lifecycle_runs:$name SET name = $name, due = time::now();",
                        name_param(name),
                    )
                    .await?;
                parameters.push(("rules".to_owned(), rules));
                self.pool
                    .run("UPDATE buckets:$name SET lifecycle = $rules;", parameters)
                    .await?;
            }
            None => {
                self.pool
                    .run("UPDATE buckets:$name SET lifecycle = NONE;", parameters)
                    .await?;
                self.regional
                    .run("DELETE lifecycle_runs:$name;", name_param(name))
                    .await?;
            }
        }
        Ok(())
    }

    async fn set_notification(
        &self,
        name: &BucketName,
        configuration: Option<Value>,
    ) -> Result<()> {
        let mut parameters = name_param(name);
        let script = match configuration {
            Some(configuration) => {
                parameters.push(("configuration".to_owned(), configuration));
                "UPDATE buckets:$name SET notify = $configuration;"
            }
            None => "UPDATE buckets:$name SET notify = NONE;",
        };
        self.pool.run(script, parameters).await?;
        Ok(())
    }

    async fn set_media(&self, name: &BucketName, rules: Option<Value>) -> Result<()> {
        let mut parameters = name_param(name);
        let script = match rules {
            Some(rules) => {
                parameters.push(("rules".to_owned(), rules));
                "UPDATE buckets:$name SET media = $rules;"
            }
            None => "UPDATE buckets:$name SET media = NONE;",
        };
        self.pool.run(script, parameters).await?;
        Ok(())
    }

    async fn set_acknowledge(&self, name: &BucketName, local: bool) -> Result<()> {
        let mut parameters = name_param(name);
        let script = if local {
            parameters.push(("level".to_owned(), Value::String(LOCAL.to_owned())));
            "UPDATE buckets:$name SET acknowledge = $level;"
        } else {
            "UPDATE buckets:$name SET acknowledge = NONE;"
        };
        self.pool.run(script, parameters).await?;
        Ok(())
    }

    async fn probe_local(&self) -> Result<Option<String>> {
        match self
            .regional
            .run(
                "UPSERT acknowledge_probes:'region' SET checked = time::now() ACKNOWLEDGE LOCAL MAJORITY;",
                Vec::new(),
            )
            .await
        {
            Ok(_) => Ok(None),
            Err(MetaError::Refused {
                message,
                class: Some(RefusalClass::Invalid | RefusalClass::Conflict),
            }) => Ok(Some(message)),
            Err(error) => Err(error.into()),
        }
    }

    async fn set_limits(
        &self,
        name: &BucketName,
        max_bytes: Option<i64>,
        max_objects: Option<i64>,
    ) -> Result<()> {
        let limit = |value: Option<i64>| {
            value.map_or(Value::None, |limit| Value::Number(Number::Integer(limit)))
        };
        let mut parameters = name_param(name);
        parameters.push(("max_bytes".to_owned(), limit(max_bytes)));
        parameters.push(("max_objects".to_owned(), limit(max_objects)));
        // `SET`, never the whole record: only the two limits change, and NONE removes the field.
        self.pool
            .run(
                "UPDATE buckets:$name SET max_bytes = $max_bytes, max_objects = $max_objects;",
                parameters,
            )
            .await?;
        Ok(())
    }

    async fn list(&self, visible: &Visible) -> Result<Vec<BucketEntity>> {
        let answers = match visible {
            Visible::All => self.pool.run("SELECT * FROM buckets;", Vec::new()).await?,
            Visible::Space(space) => {
                let parameters =
                    vec![("space".to_owned(), Value::String(space.as_str().to_owned()))];
                self.pool
                    .run(
                        "SELECT * FROM buckets WHERE space_name = $space USING INDEX by_space;",
                        parameters,
                    )
                    .await?
            }
            Visible::Reachable {
                space,
                user,
                granted,
            } => {
                let parameters = vec![
                    ("space".to_owned(), Value::String(space.as_str().to_owned())),
                    ("user".to_owned(), Value::String(user.clone())),
                    (
                        "granted".to_owned(),
                        Value::Array(granted.iter().cloned().map(Value::String).collect()),
                    ),
                ];
                self.pool
                    .run(
                        "SELECT * FROM buckets WHERE space_name = $space AND (creator = $user OR name IN $granted) \
                         USING INDEX by_space;",
                        parameters,
                    )
                    .await?
            }
        };
        match answers.into_iter().next() {
            Some(Answer::Records { records, .. }) => records
                .iter()
                .map(|(_, value)| BucketEntity::from_value(value))
                .collect(),
            _ => Err(Error::Malformed {
                record: "bucket",
                reason: "a listing answered no records",
            }),
        }
    }

    async fn holds_objects(&self, name: &BucketName) -> Result<bool> {
        let anchor = format!("{}/", name.as_str());
        // A version's id starts with the bucket's bytes and `/` too (ADR-0010), so the first version after that anchor
        // is the bucket's own when it holds any — a delete marker or a noncurrent version keeps the bucket non-empty.
        let parameters = vec![
            (
                "versions_at".to_owned(),
                Value::Bytes(anchor.clone().into_bytes()),
            ),
            ("anchor".to_owned(), Value::String(anchor)),
        ];
        let answers = self
            .regional
            .run(
                "SELECT bucket_name FROM objects AFTER objects:$anchor LIMIT 1; \
                 SELECT bucket_name FROM versions AFTER versions:$versions_at LIMIT 1;",
                parameters,
            )
            .await?;
        let holds = |answer| -> Result<bool> {
            Ok(match first_record(answer)? {
                Some(Value::Object(fields)) => {
                    matches!(fields.get("bucket_name"), Some(Value::String(bucket)) if bucket == name.as_str())
                }
                _ => false,
            })
        };
        let mut answers = answers.into_iter();
        Ok(holds(answers.next())? || holds(answers.next())?)
    }

    async fn remove(&self, name: &BucketName) -> Result<Option<BucketEntity>> {
        let answers = self
            .pool
            .run("DELETE buckets:$name RETURN BEFORE;", name_param(name))
            .await?;
        single(answers)?
            .map(|value| BucketEntity::from_value(&value))
            .transpose()
    }
}
