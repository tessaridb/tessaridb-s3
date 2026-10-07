//! An `objects` record exactly as stored, and the value written for one.

use std::collections::BTreeMap;

use tessari_s3_infrastructure::tessaridb::{Number, Value};

use super::model::{Content, NewObject, ObjectSummary, StoredObject};
use super::repository::ListRow;
use crate::{Error, Result};

fn malformed(reason: &'static str) -> Error {
    Error::Malformed {
        record: "object",
        reason,
    }
}

fn strings(map: &BTreeMap<String, String>) -> Value {
    Value::Object(
        map.iter()
            .map(|(k, v)| (k.clone(), Value::String(v.clone())))
            .collect(),
    )
}

fn read_strings(value: Option<&Value>, field: &'static str) -> Result<BTreeMap<String, String>> {
    let Some(Value::Object(map)) = value else {
        return Err(malformed(field));
    };
    map.iter()
        .map(|(k, v)| match v {
            Value::String(text) => Ok((k.clone(), text.clone())),
            _ => Err(malformed(field)),
        })
        .collect()
}

/// The record value for `object` in `bucket` under `incarnation`; `modified` is the store's clock, set in the
/// statement.
pub(crate) fn record(
    bucket: &str,
    key: &str,
    incarnation: [u8; 16],
    object: &NewObject,
) -> Result<Value> {
    let size = i64::try_from(object.size).map_err(|_| malformed("size beyond i64"))?;
    let content = match &object.content {
        Content::Inline(bytes) => ("inline", Value::Bytes(bytes.clone())),
        Content::Data(id) => ("data", Value::Uuid(*id)),
    };
    let fields: BTreeMap<String, Value> = [
        ("bucket_name", Value::String(bucket.to_owned())),
        ("key", Value::String(key.to_owned())),
        ("incarnation", Value::Uuid(incarnation)),
        ("size", Value::from(size)),
        ("etag", Value::String(object.etag.clone())),
        ("headers", strings(&object.headers)),
        ("metadata", strings(&object.metadata)),
        ("checksums", strings(&object.checksums)),
        content,
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect();
    Ok(Value::Object(fields))
}

type Fields = BTreeMap<String, Value>;

fn incarnation(fields: &Fields) -> Result<[u8; 16]> {
    match fields.get("incarnation") {
        Some(Value::Uuid(bytes)) => Ok(*bytes),
        _ => Err(malformed("incarnation")),
    }
}

fn size(fields: &Fields) -> Result<u64> {
    match fields.get("size") {
        Some(Value::Number(Number::Integer(size))) => {
            u64::try_from(*size).map_err(|_| malformed("size"))
        }
        _ => Err(malformed("size")),
    }
}

fn text(fields: &Fields, field: &'static str) -> Result<String> {
    match fields.get(field) {
        Some(Value::String(text)) => Ok(text.clone()),
        _ => Err(malformed(field)),
    }
}

fn modified(fields: &Fields) -> Result<tessari_s3_types::Timestamp> {
    match fields.get("modified") {
        Some(Value::Datetime { seconds, nanos }) => Ok(tessari_s3_types::Timestamp {
            seconds: *seconds,
            nanos: *nanos,
        }),
        _ => Err(malformed("modified")),
    }
}

fn fields_of(value: &Value) -> Result<&Fields> {
    match value {
        Value::Object(fields) => Ok(fields),
        _ => Err(malformed("not an object")),
    }
}

/// A stored record's incarnation and the object it holds.
pub(crate) fn read(value: &Value) -> Result<([u8; 16], StoredObject)> {
    let fields = fields_of(value)?;
    let content = match (fields.get("data"), fields.get("inline")) {
        (Some(Value::Uuid(id)), _) => Content::Data(*id),
        (_, Some(Value::Bytes(bytes))) => Content::Inline(bytes.clone()),
        _ => return Err(malformed("neither inline bytes nor a data id")),
    };
    let object = StoredObject {
        size: size(fields)?,
        etag: text(fields, "etag")?,
        modified: modified(fields)?,
        headers: read_strings(fields.get("headers"), "headers")?,
        metadata: read_strings(fields.get("metadata"), "metadata")?,
        checksums: read_strings(fields.get("checksums"), "checksums")?,
        content,
    };
    Ok((incarnation(fields)?, object))
}

/// A record as a listing reads it (`bucket_name, key, incarnation, size, etag, modified, checksums`).
pub(crate) fn list_row(value: &Value) -> Result<ListRow> {
    let fields = fields_of(value)?;
    Ok(ListRow {
        bucket: text(fields, "bucket_name")?,
        key: text(fields, "key")?,
        incarnation: incarnation(fields)?,
        summary: ObjectSummary {
            size: size(fields)?,
            etag: text(fields, "etag")?,
            modified: modified(fields)?,
            checksum_algorithms: read_strings(fields.get("checksums"), "checksums")?
                .into_keys()
                .collect(),
        },
    })
}
