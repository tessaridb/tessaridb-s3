//! Objects in TessariDB: one `objects` record per key, id `<bucket>/<key>`; every value is bound.

use tessari_s3_infrastructure::tessaridb::{Answer, MetaError, MetaPool, RefusalClass, Value};
use tessari_s3_types::{BucketName, ObjectKey};

use super::entity::{read, record};
use super::model::{Content, NewObject};
use super::repository::{Guard, ObjectRepository, Snapshot, Wrote};
use crate::{Error, Result};

/// The TessariDB object repository.
#[derive(Clone)]
pub struct TessariObjects {
    pool: MetaPool,
}

impl TessariObjects {
    /// A repository over `pool`.
    #[must_use]
    pub const fn new(pool: MetaPool) -> Self {
        Self { pool }
    }
}

fn id(bucket: &BucketName, key: &ObjectKey) -> Value {
    Value::String(format!("{}/{}", bucket.as_str(), key.as_str()))
}

/// The first record of a records answer.
fn first_record(answer: Option<Answer>) -> Result<Option<Value>> {
    match answer {
        Some(Answer::Records { records, .. }) => {
            Ok(records.into_iter().next().map(|(_, value)| value))
        }
        Some(Answer::Value {
            value: Value::None, ..
        })
        | None => Ok(None),
        Some(Answer::Value { value, .. }) => Ok(Some(value)),
        Some(_) => Err(Error::Malformed {
            record: "object",
            reason: "unexpected answer kind",
        }),
    }
}

fn incarnation_of(value: Option<Value>) -> Result<Option<[u8; 16]>> {
    match value {
        None => Ok(None),
        Some(Value::Object(fields)) => match fields.get("incarnation") {
            Some(Value::Uuid(bytes)) => Ok(Some(*bytes)),
            _ => Err(Error::Malformed {
                record: "bucket",
                reason: "incarnation",
            }),
        },
        Some(_) => Err(Error::Malformed {
            record: "bucket",
            reason: "not an object",
        }),
    }
}

/// A refusal that means the statement's own condition or target failed: `conflict` (the compare-and-set) or
/// `invalid` (no record to update). Retry and outage classes are not this.
fn is_condition_refusal(error: &MetaError) -> bool {
    error.is_class(RefusalClass::Conflict) || error.is_class(RefusalClass::Invalid)
}

impl ObjectRepository for TessariObjects {
    async fn incarnation(&self, bucket: &BucketName) -> Result<Option<[u8; 16]>> {
        let parameters = vec![(
            "bucket".to_owned(),
            Value::String(bucket.as_str().to_owned()),
        )];
        let answers = self
            .pool
            .run("SELECT incarnation FROM ONLY buckets:$bucket;", parameters)
            .await?;
        incarnation_of(first_record(answers.into_iter().next())?)
    }

    async fn read(&self, bucket: &BucketName, key: &ObjectKey) -> Result<Snapshot> {
        let parameters = vec![
            (
                "bucket".to_owned(),
                Value::String(bucket.as_str().to_owned()),
            ),
            ("id".to_owned(), id(bucket, key)),
        ];
        // One transaction, so the bucket's incarnation and the object are read at one snapshot.
        let script = "BEGIN; SELECT incarnation FROM ONLY buckets:$bucket; SELECT * FROM ONLY objects:$id; COMMIT;";
        let mut answers = self.pool.run(script, parameters).await?.into_iter();
        let _begin = answers.next();
        let bucket_incarnation = incarnation_of(first_record(answers.next())?)?;
        let object = first_record(answers.next())?
            .map(|value| read(&value))
            .transpose()?;
        Ok(Snapshot {
            bucket: bucket_incarnation,
            object,
        })
    }

    async fn write(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        incarnation: [u8; 16],
        object: &NewObject,
        guard: &Guard,
    ) -> Result<Wrote> {
        let mut parameters = vec![
            ("id".to_owned(), id(bucket, key)),
            (
                "object".to_owned(),
                record(bucket.as_str(), key.as_str(), incarnation, object)?,
            ),
            ("incarnation".to_owned(), Value::Uuid(incarnation)),
        ];
        // `modified` is the store's clock at commit, merged into the bound record.
        let statement = match guard {
            Guard::None => {
                "UPSERT objects:$id = object::merge($object, { modified: time::now() }) RETURN AFTER;"
            }
            Guard::Absent => {
                "CREATE objects:$id = object::merge($object, { modified: time::now() }) RETURN AFTER;"
            }
            Guard::Etag(etag) => {
                parameters.push(("etag".to_owned(), Value::String(etag.clone())));
                "UPDATE objects:$id = object::merge($object, { modified: time::now() }) \
                 WHERE etag = $etag AND incarnation = $incarnation RETURN AFTER;"
            }
            Guard::Incarnation(stale) => {
                parameters.push(("stale".to_owned(), Value::Uuid(*stale)));
                "UPDATE objects:$id = object::merge($object, { modified: time::now() }) \
                 WHERE incarnation = $stale RETURN AFTER;"
            }
        };
        // A data object commits together with the removal of its pending `gc` entry; the `supersede` event queues
        // whatever data the record held before, in the same transaction. The write's answer follows `BEGIN`.
        let (script, answer_at) = match &object.content {
            Content::Inline(_) => (statement.to_owned(), 0),
            Content::Data(data) => {
                parameters.push(("data".to_owned(), Value::Uuid(*data)));
                (format!("BEGIN; {statement} DELETE gc:$data; COMMIT;"), 1)
            }
        };
        match self.pool.run(&script, parameters).await {
            Ok(answers) => match first_record(answers.into_iter().nth(answer_at))? {
                Some(value) => Ok(Wrote::Record(value)),
                None => Err(Error::Malformed {
                    record: "object",
                    reason: "a write answered no record",
                }),
            },
            Err(error) if is_condition_refusal(&error) => Ok(Wrote::Refused),
            Err(error) => Err(error.into()),
        }
    }

    async fn queue(&self, id: [u8; 16]) -> Result<()> {
        let parameters = vec![("data".to_owned(), Value::Uuid(id))];
        self.pool
            .run(
                "UPSERT gc:$data SET data = $data, queued = time::now();",
                parameters,
            )
            .await?;
        Ok(())
    }

    async fn unqueue(&self, id: [u8; 16]) -> Result<()> {
        let parameters = vec![("data".to_owned(), Value::Uuid(id))];
        self.pool.run("DELETE gc:$data;", parameters).await?;
        Ok(())
    }

    async fn remove(&self, bucket: &BucketName, key: &ObjectKey) -> Result<()> {
        let parameters = vec![("id".to_owned(), id(bucket, key))];
        self.pool.run("DELETE objects:$id;", parameters).await?;
        Ok(())
    }
}
