//! Buckets in TessariDB: one `buckets` record per bucket, id = the bucket name; every value is bound.

use tessari_s3_infrastructure::tessaridb::{Answer, MetaPool, RefusalClass, Value};
use tessari_s3_types::BucketName;

use super::entity::BucketEntity;
use super::repository::{BucketRepository, Inserted};
use crate::{Error, Result};

/// The TessariDB bucket repository.
#[derive(Clone)]
pub struct TessariBuckets {
    pool: MetaPool,
}

impl TessariBuckets {
    /// A repository over `pool`.
    #[must_use]
    pub const fn new(pool: MetaPool) -> Self {
        Self { pool }
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
    async fn insert(&self, name: &BucketName, region: &str) -> Result<Inserted> {
        let mut parameters = name_param(name);
        parameters.push(("region".to_owned(), Value::String(region.to_owned())));
        let script = "CREATE buckets:$name = { name: $name, created: time::now(), region: $region, \
                      incarnation: rand::uuid() } RETURN AFTER;";
        match self.pool.run(script, parameters).await {
            Ok(answers) => match single(answers)? {
                Some(value) => Ok(Inserted::Created(BucketEntity::from_value(&value)?)),
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

    async fn list(&self) -> Result<Vec<BucketEntity>> {
        let answers = self.pool.run("SELECT * FROM buckets;", Vec::new()).await?;
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
        let parameters = vec![("anchor".to_owned(), Value::String(anchor))];
        let answers = self
            .pool
            .run(
                "SELECT bucket_name FROM objects AFTER objects:$anchor LIMIT 1;",
                parameters,
            )
            .await?;
        Ok(match single(answers)? {
            Some(Value::Object(fields)) => {
                matches!(fields.get("bucket_name"), Some(Value::String(bucket)) if bucket == name.as_str())
            }
            _ => false,
        })
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
