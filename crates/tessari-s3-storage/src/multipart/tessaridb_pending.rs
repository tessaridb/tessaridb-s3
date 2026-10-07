//! The order ListMultipartUploads walks: one `pending` record per open upload, whose id is
//! `<bucket>/<hex of the key's bytes>-<initiated, nanoseconds, 20 digits>-<upload id>`. Hex keeps the keys' byte order
//! and their prefixes with no escaping (a key may hold any byte); `-` sorts below every hex digit, so all of one key's
//! uploads precede every longer key; within a key the initiation instant and then the id decide. A walk is `AFTER` an
//! anchor that need not exist, with no `ORDER BY`, so it seeks.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use tessari_s3_core::objects::upload_listing::UploadAnchor;
use tessari_s3_infrastructure::tessaridb::{Answer, MetaPool, Value};
use tessari_s3_types::{BucketName, ObjectKey, Timestamp, UploadId};

use super::super::model::{NewUpload, OpenUpload};
use super::super::repository::{PendingBatch, PendingRow};
use super::strings;
use crate::answers::{first_record, incarnation_of};
use crate::{Error, Result};

fn malformed(reason: &'static str) -> Error {
    Error::Malformed {
        record: "pending",
        reason,
    }
}

/// Lowercase hex of `bytes`.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(
        String::with_capacity(bytes.len().saturating_mul(2)),
        |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        },
    )
}

/// Where an upload of `key` started at `initiated` sits in the listing order.
pub(super) fn position(
    bucket: &BucketName,
    key: &ObjectKey,
    initiated: Timestamp,
    id: UploadId,
) -> Result<String> {
    let seconds =
        u64::try_from(initiated.seconds).map_err(|_| malformed("initiated before 1970"))?;
    // 20 digits hold every instant before the year 5138, so the digits order as the instants do.
    let nanos = u128::from(seconds)
        .checked_mul(1_000_000_000)
        .and_then(|n| n.checked_add(u128::from(initiated.nanos)))
        .ok_or_else(|| malformed("initiated"))?;
    Ok(format!(
        "{}/{}-{nanos:020}-{}",
        bucket.as_str(),
        hex(key.as_str().as_bytes()),
        id.hex()
    ))
}

/// The position a walk from `anchor` starts after (exclusive; it need not be a record).
pub(super) fn position_of(bucket: &BucketName, anchor: &UploadAnchor<OpenUpload>) -> String {
    let in_bucket = |rest: String| format!("{}/{rest}", bucket.as_str());
    match anchor {
        // No position is a bare key: every one continues with `-`, so the first upload of `key` follows.
        UploadAnchor::From(key) => in_bucket(hex(key.as_bytes())),
        // `-g` sorts after every `-<digits>` of this key and before every longer key's hex digit.
        UploadAnchor::PastKey(key) => in_bucket(format!("{}-g", hex(key.as_bytes()))),
        // `g` sorts after every hex digit and `-`: past every key under the prefix.
        UploadAnchor::PastPrefix(prefix) => in_bucket(format!("{}g", hex(prefix.as_bytes()))),
        UploadAnchor::After(upload) => upload.position.clone(),
    }
}

/// See `MultipartRepository::create`.
pub(super) async fn create(
    pool: &MetaPool,
    id: UploadId,
    bucket: &BucketName,
    key: &ObjectKey,
    incarnation: [u8; 16],
    initiated: Timestamp,
    upload: &NewUpload,
) -> Result<()> {
    let listed = position(bucket, key, initiated, id)?;
    let when = Value::Datetime {
        seconds: initiated.seconds,
        nanos: initiated.nanos,
    };
    let common = [
        ("bucket_name", Value::String(bucket.as_str().to_owned())),
        ("key", Value::String(key.as_str().to_owned())),
        ("incarnation", Value::Uuid(incarnation)),
        ("initiated", when),
        ("position", Value::String(listed.clone())),
    ];
    let mut record: BTreeMap<String, Value> = common
        .iter()
        .cloned()
        .chain([
            ("headers", strings(&upload.headers)),
            ("metadata", strings(&upload.metadata)),
        ])
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
    for (field, value) in [
        ("checksum_algorithm", &upload.checksum_algorithm),
        ("checksum_type", &upload.checksum_type),
    ] {
        if let Some(value) = value {
            record.insert(field.to_owned(), Value::String(value.clone()));
        }
    }
    let pending: BTreeMap<String, Value> = common
        .into_iter()
        .chain([("upload", Value::Uuid(id.bytes()))])
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
    let parameters = vec![
        ("upload".to_owned(), Value::Uuid(id.bytes())),
        ("record".to_owned(), Value::Object(record)),
        ("listed".to_owned(), Value::String(listed)),
        ("pending".to_owned(), Value::Object(pending)),
    ];
    pool.run(
        "BEGIN; CREATE uploads:$upload = $record; CREATE pending:$listed = $pending; COMMIT;",
        parameters,
    )
    .await?;
    Ok(())
}

/// A `pending` record as the walk reads it.
fn pending_row(value: &Value) -> Result<PendingRow> {
    let Value::Object(fields) = value else {
        return Err(malformed("pending"));
    };
    let text = |field: &'static str| match fields.get(field) {
        Some(Value::String(text)) => Ok(text.clone()),
        _ => Err(malformed(field)),
    };
    let uuid = |field: &'static str| match fields.get(field) {
        Some(Value::Uuid(bytes)) => Ok(*bytes),
        _ => Err(malformed(field)),
    };
    Ok(PendingRow {
        bucket: text("bucket_name")?,
        key: text("key")?,
        incarnation: uuid("incarnation")?,
        upload: OpenUpload {
            id: UploadId::from_bytes(uuid("upload")?),
            initiated: initiated_of(fields.get("initiated"))?,
            position: text("position")?,
        },
    })
}

pub(super) fn initiated_of(value: Option<&Value>) -> Result<Timestamp> {
    match value {
        Some(Value::Datetime { seconds, nanos }) => Ok(Timestamp {
            seconds: *seconds,
            nanos: *nanos,
        }),
        _ => Err(malformed("initiated")),
    }
}

/// See `MultipartRepository::pending`.
pub(super) async fn page(
    pool: &MetaPool,
    bucket: &BucketName,
    after: &str,
    limit: usize,
) -> Result<PendingBatch> {
    let parameters = vec![
        (
            "bucket".to_owned(),
            Value::String(bucket.as_str().to_owned()),
        ),
        ("after".to_owned(), Value::String(after.to_owned())),
    ];
    // `LIMIT` takes a literal on 0.33.1; the limit is this server's own integer, never a caller's value.
    let script = format!(
        "BEGIN; SELECT incarnation FROM ONLY buckets:$bucket; \
         SELECT upload, bucket_name, key, incarnation, initiated, position FROM pending AFTER pending:$after \
         LIMIT {limit}; COMMIT;"
    );
    let mut answers = pool.run(&script, parameters).await?.into_iter();
    let _begin = answers.next();
    let bucket_incarnation = incarnation_of(first_record(answers.next())?)?;
    let walked = match answers.next() {
        Some(Answer::Records { records, .. }) => records,
        _ => Vec::new(),
    };
    let full = walked.len() >= limit;
    let rows = walked
        .iter()
        .map(|(_, value)| pending_row(value))
        .collect::<Result<Vec<_>>>()?;
    Ok(PendingBatch {
        bucket: bucket_incarnation,
        rows,
        full,
    })
}

/// See `MultipartRepository::initiated_before`. The index range is exactly the uploads due, so walking it is the work.
pub(super) async fn initiated_before(
    pool: &MetaPool,
    cutoff: Timestamp,
    limit: usize,
) -> Result<Vec<UploadId>> {
    let parameters = vec![(
        "cutoff".to_owned(),
        Value::Datetime {
            seconds: cutoff.seconds,
            nanos: cutoff.nanos,
        },
    )];
    // `LIMIT` takes a literal on 0.33.1; the limit is this server's own integer, never a caller's value.
    let script = format!(
        "SELECT upload FROM pending WHERE initiated < $cutoff LIMIT {limit} USING INDEX by_pending_initiated;"
    );
    let answers = pool.run(&script, parameters).await?;
    let Some(Answer::Records { records, .. }) = answers.into_iter().next() else {
        return Ok(Vec::new());
    };
    records
        .iter()
        .map(|(_, value)| match value {
            Value::Object(fields) => match fields.get("upload") {
                Some(Value::Uuid(id)) => Ok(UploadId::from_bytes(*id)),
                _ => Err(malformed("upload")),
            },
            _ => Err(malformed("not an object")),
        })
        .collect()
}
