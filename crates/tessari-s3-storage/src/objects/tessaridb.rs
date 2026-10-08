//! Objects in TessariDB: one `objects` record per key, id `<bucket>/<key>`; every value is bound.

use tessari_s3_infrastructure::tessaridb::{Answer, MetaError, MetaPool, RefusalClass, Value};
use tessari_s3_types::{BucketName, ObjectKey};

use super::entity::{read, record};
use super::model::{Content, NewObject, Part};
use super::repository::{Batch, Guard, ObjectRepository, Snapshot, Wrote};
use crate::answers::{first_record, incarnation_of};
use crate::{Error, Result};

#[path = "tessaridb_page.rs"]
mod page;
#[path = "tessaridb_parts.rs"]
mod parts;
#[path = "tessaridb_remove.rs"]
mod remove;

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
        // A data object commits together with the removal of its pending `gc` entry, and only while that entry exists
        // unmarked — the `UPDATE` refuses, and the whole transaction with it, once a reclaimer has taken the file. The
        // `supersede` event queues whatever data the record held before, in the same transaction. The write's answer
        // follows `BEGIN`.
        let (script, answer_at) = match &object.content {
            Content::Inline(_) => (statement.to_owned(), 0),
            // A completed upload: the upload must still exist (the `UPDATE` refuses the whole transaction once an
            // abort or another completion took it), the unlisted parts go (their event queues their files), and the
            // upload ends — all in the commit that makes the object visible.
            Content::Parts(multipart) => {
                let listed = multipart
                    .parts
                    .iter()
                    .map(|part| Value::from(i64::from(part.number)))
                    .collect();
                parameters.push(("upload".to_owned(), Value::Uuid(multipart.upload)));
                parameters.push(("listed".to_owned(), Value::Array(listed)));
                (
                    format!(
                        "BEGIN; UPDATE uploads:$upload SET last_part = time::now(); {statement} \
                         DELETE FROM parts WHERE upload = $upload AND NOT (number IN $listed) LIMIT ALL; \
                         DELETE uploads:$upload; COMMIT;"
                    ),
                    2,
                )
            }
            Content::Data(data) => {
                parameters.push(("data".to_owned(), Value::Uuid(*data)));
                (
                    format!(
                        "BEGIN; {statement} UPDATE gc:$data SET data = $data WHERE reclaiming = NONE; \
                         DELETE gc:$data; COMMIT;"
                    ),
                    1,
                )
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

    async fn due(&self, grace_secs: u64, limit: usize) -> Result<Vec<([u8; 16], bool)>> {
        let number = |value: u64| {
            i64::try_from(value).map_err(|_| Error::Malformed {
                record: "gc",
                reason: "a bound beyond i64",
            })
        };
        let parameters = vec![("grace".to_owned(), Value::from(number(grace_secs)?))];
        // The cutoff is computed by the store, from the same clock that stamped `queued`. `LIMIT` takes a literal
        // (0.33.1 refuses a parameter there); the limit is this server's own integer, never a caller's value, and an
        // integer formats to digits only.
        let script = format!(
            "SELECT data, reclaiming FROM gc \
             WHERE time::unix(queued) <= time::unix(time::now()) - $grace LIMIT {limit};"
        );
        let answers = self.pool.run(&script, parameters).await?;
        let Some(Answer::Records { records, .. }) = answers.into_iter().next() else {
            return Ok(Vec::new());
        };
        records
            .into_iter()
            .map(|(_, record)| match record {
                Value::Object(fields) => match fields.get("data") {
                    Some(Value::Uuid(id)) => Ok((
                        *id,
                        matches!(fields.get("reclaiming"), Some(Value::Datetime { .. })),
                    )),
                    _ => Err(Error::Malformed {
                        record: "gc",
                        reason: "data",
                    }),
                },
                _ => Err(Error::Malformed {
                    record: "gc",
                    reason: "not an object",
                }),
            })
            .collect()
    }

    async fn referenced(&self, id: [u8; 16]) -> Result<bool> {
        let parameters = vec![("data".to_owned(), Value::Uuid(id))];
        // An object or a multipart part may hold the file. `USING` makes a dropped or renamed index a refusal here
        // rather than a full scan.
        let script = "SELECT key FROM objects WHERE data = $data LIMIT 1 USING INDEX by_data; \
                      SELECT number FROM parts WHERE data = $data LIMIT 1 USING INDEX by_part_data;";
        let mut answers = self.pool.run(script, parameters).await?.into_iter();
        let by_object = first_record(answers.next())?.is_some();
        Ok(by_object || first_record(answers.next())?.is_some())
    }

    async fn mark(&self, id: [u8; 16]) -> Result<bool> {
        let parameters = vec![("data".to_owned(), Value::Uuid(id))];
        let script =
            "UPDATE gc:$data SET reclaiming = time::now() WHERE reclaiming = NONE RETURN AFTER;";
        match self.pool.run(script, parameters).await {
            Ok(answers) => Ok(first_record(answers.into_iter().next())?.is_some()),
            Err(error) if is_condition_refusal(&error) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    async fn claimable(&self, id: [u8; 16]) -> Result<bool> {
        let parameters = vec![("data".to_owned(), Value::Uuid(id))];
        let answers = self
            .pool
            .run("SELECT reclaiming FROM ONLY gc:$data;", parameters)
            .await?;
        Ok(match first_record(answers.into_iter().next())? {
            Some(Value::Object(fields)) => {
                !matches!(fields.get("reclaiming"), Some(Value::Datetime { .. }))
            }
            _ => false,
        })
    }

    async fn page(
        &self,
        bucket: &BucketName,
        at: Option<&str>,
        after: &str,
        limit: usize,
    ) -> Result<Batch> {
        page::page(&self.pool, bucket, at, after, limit).await
    }

    async fn parts_of(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        upload: [u8; 16],
    ) -> Result<Option<Vec<Part>>> {
        parts::parts_of(&self.pool, id(bucket, key), upload).await
    }

    async fn remove_if(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        incarnation: [u8; 16],
        etag: &str,
    ) -> Result<bool> {
        remove::remove_if(&self.pool, id(bucket, key), incarnation, etag).await
    }

    async fn remove(&self, bucket: &BucketName, key: &ObjectKey) -> Result<()> {
        let parameters = vec![("id".to_owned(), id(bucket, key))];
        self.pool.run("DELETE objects:$id;", parameters).await?;
        Ok(())
    }
}
