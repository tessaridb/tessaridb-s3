//! Objects in TessariDB: one `objects` record per key, id `<bucket>/<key>`; every value is bound.

use tessari_s3_constants::RECLAIM_RETRY_SECS;
use tessari_s3_infrastructure::tessaridb::{Answer, MetaError, MetaPool, RefusalClass, Value};
use tessari_s3_types::{BucketName, ObjectKey};

use super::entity::read;
use super::model::{NewObject, Part};
use super::repository::{Batch, Guard, Head, ObjectRepository, Snapshot, VersionPlan, Wrote};
use crate::answers::{bucket_read, first_record, incarnation_of};
use crate::schema::QUEUE_FOR_RECLAIM;
use crate::{Error, Result};

#[path = "tessaridb_convert.rs"]
mod convert;
#[path = "tessaridb_convert_versions.rs"]
mod convert_versions;
#[path = "tessaridb_page.rs"]
mod page;
#[path = "tessaridb_parts.rs"]
mod parts;
#[path = "tessaridb_remove.rs"]
mod remove;
#[path = "tessaridb_replica.rs"]
mod replica;
#[path = "tessaridb_version_delete.rs"]
mod version_delete;
#[path = "tessaridb_version_list.rs"]
mod version_list;
#[path = "tessaridb_version_lock.rs"]
mod version_lock;
#[path = "tessaridb_version_read.rs"]
mod version_read;

pub(crate) use version_read::Found;
#[path = "tessaridb_versions.rs"]
mod versions;
#[path = "tessaridb_write.rs"]
mod write;

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

/// A refusal that means the statement's own condition or target failed: the compare-and-set, a record already there,
/// or no record to update — all `conflict` from TessariDB 0.33.4. An `invalid` refusal (a broken event, a schema the
/// write does not fit) is a fault of this server and is never read as a condition; retry and outage are not this.
fn is_condition_refusal(error: &MetaError) -> bool {
    error.is_class(RefusalClass::Conflict)
}

/// A refusal of a write that changes a key's versions: its own condition, or a writer of the same key that committed
/// first — the store answers that collision `retry` (TessariDB ref 05 M6), and the caller plans the write again.
fn is_lost_or_refused(error: &MetaError) -> bool {
    is_condition_refusal(error) || error.is_class(RefusalClass::Retry)
}

/// The refusal of the store's own Object Lock events (`locked_release`, `locked_change`): the version became held or
/// retained after it was read, so the caller reads it again and answers that it is kept (ADR-0016).
fn is_lock_refusal(error: &MetaError) -> bool {
    matches!(error, MetaError::Refused { message, class: Some(RefusalClass::Invalid) } if message.contains("ObjectLocked:"))
}

impl ObjectRepository for TessariObjects {
    async fn incarnation(&self, bucket: &BucketName) -> Result<Option<[u8; 16]>> {
        let parameters = vec![(
            "bucket".to_owned(),
            Value::String(bucket.as_str().to_owned()),
        )];
        let answers = self
            .pool
            .run(&bucket_read(&self.pool, "incarnation, region"), parameters)
            .await?;
        incarnation_of(
            first_record(answers.into_iter().next())?,
            self.pool.region(),
        )
    }

    async fn read(&self, bucket: &BucketName, key: &ObjectKey) -> Result<Snapshot> {
        let parameters = vec![
            (
                "bucket".to_owned(),
                Value::String(bucket.as_str().to_owned()),
            ),
            ("id".to_owned(), id(bucket, key)),
        ];
        // Two plain reads, the bucket first: an object is taken only when it carries the incarnation read just before
        // it, and every write is fenced on that incarnation in its own commit, so the pair answers as of one moment.
        let script = format!(
            "{} SELECT * FROM ONLY objects:$id;",
            bucket_read(&self.pool, "incarnation, region")
        );
        let mut answers = self.pool.run(&script, parameters).await?.into_iter();
        let bucket_incarnation = incarnation_of(first_record(answers.next())?, self.pool.region())?;
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
        versions: &VersionPlan,
    ) -> Result<Wrote> {
        write::write(
            &self.pool,
            bucket,
            key,
            incarnation,
            object,
            guard,
            versions,
        )
        .await
    }

    async fn queue(&self, id: [u8; 16]) -> Result<()> {
        let parameters = vec![("data".to_owned(), Value::Uuid(id))];
        self.pool.run(QUEUE_FOR_RECLAIM, parameters).await?;
        Ok(())
    }

    async fn unqueue(&self, id: [u8; 16]) -> Result<()> {
        let parameters = vec![("data".to_owned(), Value::Uuid(id))];
        self.pool.run("DELETE reclaims:$data;", parameters).await?;
        Ok(())
    }

    async fn claim_next(&self) -> Result<Option<([u8; 16], bool)>> {
        let answers = self.pool.run("CLAIM FROM reclaims;", Vec::new()).await?;
        let Some(Answer::Records { records, .. }) = answers.into_iter().next() else {
            return Ok(None);
        };
        let Some((_, record)) = records.into_iter().next() else {
            return Ok(None);
        };
        let malformed = |reason| Error::Malformed {
            record: "reclaims",
            reason,
        };
        let Value::Object(fields) = record else {
            return Err(malformed("not an object"));
        };
        match fields.get("data") {
            Some(Value::Uuid(id)) => Ok(Some((
                *id,
                matches!(fields.get("reclaiming"), Some(Value::Datetime { .. })),
            ))),
            _ => Err(malformed("data")),
        }
    }

    async fn defer(&self, id: [u8; 16]) -> Result<()> {
        let parameters = vec![
            ("data".to_owned(), Value::Uuid(id)),
            (
                "retry".to_owned(),
                crate::params::seconds(RECLAIM_RETRY_SECS),
            ),
        ];
        self.pool
            .run(
                "RELEASE reclaims:$data NOT BEFORE duration::from_secs($retry);",
                parameters,
            )
            .await?;
        Ok(())
    }

    async fn referenced(&self, id: [u8; 16]) -> Result<bool> {
        let parameters = vec![("data".to_owned(), Value::Uuid(id))];
        // An object, any version of one, or a multipart part may hold the file. `USING` makes a dropped or renamed
        // index a refusal here rather than a full scan.
        let script = "SELECT key FROM objects WHERE data = $data LIMIT 1 USING INDEX by_data; \
                      SELECT number FROM parts WHERE data = $data LIMIT 1 USING INDEX by_part_data; \
                      SELECT seq FROM versions WHERE data = $data LIMIT 1 USING INDEX by_version_data;";
        let mut answers = self.pool.run(script, parameters).await?.into_iter();
        let by_object = first_record(answers.next())?.is_some();
        let by_part = first_record(answers.next())?.is_some();
        Ok(by_object || by_part || first_record(answers.next())?.is_some())
    }

    async fn mark(&self, id: [u8; 16]) -> Result<bool> {
        let parameters = vec![("data".to_owned(), Value::Uuid(id))];
        let script = "UPDATE reclaims:$data SET reclaiming = time::now() WHERE reclaiming = NONE RETURN AFTER;";
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
            .run("SELECT reclaiming FROM ONLY reclaims:$data;", parameters)
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
        parts::parts_of(&self.pool, "objects", id(bucket, key), upload).await
    }

    async fn remove_if(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        incarnation: [u8; 16],
        etag: &str,
        modified: Option<tessari_s3_types::Timestamp>,
        versions: &VersionPlan,
    ) -> Result<bool> {
        let guard = Some((etag, modified));
        remove::remove(&self.pool, bucket, key, incarnation, guard, versions).await
    }

    async fn remove(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        incarnation: [u8; 16],
        versions: &VersionPlan,
    ) -> Result<bool> {
        remove::remove(&self.pool, bucket, key, incarnation, None, versions).await
    }

    async fn head(&self, bucket: &BucketName, key: &ObjectKey) -> Result<Head> {
        versions::head(&self.pool, bucket, key).await
    }
}
