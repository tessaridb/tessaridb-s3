//! An `objects` record exactly as stored, and the value written for one.

use std::collections::BTreeMap;

use tessari_s3_infrastructure::tessaridb::{Number, Value};

use super::model::{Content, NewObject, StoredObject};
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

/// A stored record's incarnation and the object it holds.
pub(crate) fn read(value: &Value) -> Result<([u8; 16], StoredObject)> {
    let Value::Object(fields) = value else {
        return Err(malformed("not an object"));
    };
    let incarnation = match fields.get("incarnation") {
        Some(Value::Uuid(bytes)) => *bytes,
        _ => return Err(malformed("incarnation")),
    };
    let size = match fields.get("size") {
        Some(Value::Number(Number::Integer(size))) => {
            u64::try_from(*size).map_err(|_| malformed("size"))?
        }
        _ => return Err(malformed("size")),
    };
    let etag = match fields.get("etag") {
        Some(Value::String(etag)) => etag.clone(),
        _ => return Err(malformed("etag")),
    };
    let modified = match fields.get("modified") {
        Some(Value::Datetime { seconds, nanos }) => tessari_s3_types::Timestamp {
            seconds: *seconds,
            nanos: *nanos,
        },
        _ => return Err(malformed("modified")),
    };
    let content = match (fields.get("data"), fields.get("inline")) {
        (Some(Value::Uuid(id)), _) => Content::Data(*id),
        (_, Some(Value::Bytes(bytes))) => Content::Inline(bytes.clone()),
        _ => return Err(malformed("neither inline bytes nor a data id")),
    };
    let object = StoredObject {
        size,
        etag,
        modified,
        headers: read_strings(fields.get("headers"), "headers")?,
        metadata: read_strings(fields.get("metadata"), "metadata")?,
        checksums: read_strings(fields.get("checksums"), "checksums")?,
        content,
    };
    Ok((incarnation, object))
}
