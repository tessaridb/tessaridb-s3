//! The walk ListObjectVersions reads: `versions` in record-id order, which is the key's bytes ascending and, within a
//! key, newest first (ADR-0010). Plain reads, the bucket first; a row is shown only when it carries that incarnation.

use tessari_s3_infrastructure::tessaridb::{Answer, Number, Value};
use tessari_s3_types::{BucketName, Timestamp};

use super::super::version_id::{key_prefix, record_id};
use super::TessariObjects;
use crate::answers::{bucket_read, first_record, incarnation_of};
use crate::{Error, Result};

/// One walked version record.
pub(crate) struct VersionRow {
    pub(crate) bucket: String,
    pub(crate) key: String,
    pub(crate) incarnation: [u8; 16],
    pub(crate) at: Vec<u8>,
    pub(crate) version_id: String,
    pub(crate) marker: bool,
    pub(crate) modified: Timestamp,
    /// Size and ETag; `None` for a delete marker.
    pub(crate) object: Option<(u64, String)>,
}

/// A batch of the walk and the bucket's incarnation read before it.
pub(crate) struct VersionBatch {
    pub(crate) bucket: Option<[u8; 16]>,
    pub(crate) rows: Vec<VersionRow>,
    pub(crate) full: bool,
}

const fn malformed(reason: &'static str) -> Error {
    Error::Malformed {
        record: "versions",
        reason,
    }
}

fn text(fields: &std::collections::BTreeMap<String, Value>, field: &'static str) -> Result<String> {
    match fields.get(field) {
        Some(Value::String(text)) => Ok(text.clone()),
        _ => Err(malformed(field)),
    }
}

fn whole(fields: &std::collections::BTreeMap<String, Value>, field: &'static str) -> Result<u64> {
    match fields.get(field) {
        Some(Value::Number(Number::Integer(value))) => {
            u64::try_from(*value).map_err(|_| malformed(field))
        }
        _ => Err(malformed(field)),
    }
}

fn row(value: &Value) -> Result<VersionRow> {
    let Value::Object(fields) = value else {
        return Err(malformed("not an object"));
    };
    let key = text(fields, "key")?;
    let row_bucket = text(fields, "bucket_name")?;
    let seq = whole(fields, "seq")?;
    let marker = matches!(fields.get("marker"), Some(Value::Bool(true)));
    let modified = match fields.get("modified") {
        Some(Value::Datetime { seconds, nanos }) => Timestamp {
            seconds: *seconds,
            nanos: *nanos,
        },
        _ => return Err(malformed("modified")),
    };
    let incarnation = match fields.get("incarnation") {
        Some(Value::Uuid(id)) => *id,
        _ => return Err(malformed("incarnation")),
    };
    let object = if marker {
        None
    } else {
        Some((whole(fields, "size")?, text(fields, "etag")?))
    };
    Ok(VersionRow {
        at: record_id(&key_prefix(&row_bucket, &key), seq),
        bucket: row_bucket,
        key,
        incarnation,
        version_id: text(fields, "version_id")?,
        marker,
        modified,
        object,
    })
}

impl TessariObjects {
    /// Up to `limit` version records of the store after record id `after`, with `bucket`'s incarnation read first.
    pub(crate) async fn version_batch(
        &self,
        bucket: &BucketName,
        after: Vec<u8>,
        limit: usize,
    ) -> Result<VersionBatch> {
        let parameters = vec![
            (
                "bucket".to_owned(),
                Value::String(bucket.as_str().to_owned()),
            ),
            ("after".to_owned(), Value::Bytes(after)),
            ("limit".to_owned(), crate::params::count(limit)),
        ];
        let script = format!(
            "{} SELECT bucket_name, key, incarnation, seq, version_id, marker, modified, size, etag FROM versions \
             AFTER versions:$after LIMIT $limit;",
            bucket_read(&self.pool, "incarnation, region")
        );
        let mut answers = self.pool.run(&script, parameters).await?.into_iter();
        let incarnation = incarnation_of(first_record(answers.next())?, self.pool.region())?;
        let walked = match answers.next() {
            Some(Answer::Records { records, .. }) => records,
            _ => Vec::new(),
        };
        let full = walked.len() >= limit;
        let rows = walked
            .iter()
            .map(|(_, value)| row(value))
            .collect::<Result<Vec<_>>>()?;
        Ok(VersionBatch {
            bucket: incarnation,
            rows,
            full,
        })
    }
}
