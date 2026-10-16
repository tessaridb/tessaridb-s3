//! An `objects` record exactly as stored, and the value written for one.

use std::collections::BTreeMap;

use tessari_s3_infrastructure::tessaridb::{Number, Value};

use super::model::{Content, Multipart, NewObject, ObjectSummary, StoredObject};
use super::repository::ListRow;
use crate::encryption::WrappedKey;
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
    let content: Vec<(&str, Value)> = match &object.content {
        Content::Data(id) => vec![("data", Value::Uuid(*id))],
        Content::Parts(multipart) => {
            let count = i64::try_from(multipart.parts.len()).map_err(|_| malformed("parts"))?;
            vec![
                ("upload", Value::Uuid(multipart.upload)),
                ("parts", Value::from(count)),
            ]
        }
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
    ]
    .into_iter()
    .chain(content)
    // Only a tagged object carries the field, so an untagged record is what it always was.
    .chain((!object.tags.is_empty()).then(|| ("tags", strings(&object.tags))))
    // Likewise only a sealed object carries its wrapped data key.
    .chain(object.encryption.iter().flat_map(sealed_fields))
    .map(|(k, v)| (k.to_owned(), v))
    .collect();
    Ok(Value::Object(fields))
}

pub(crate) type Fields = BTreeMap<String, Value>;

/// The fields holding a sealed object's (or part's) wrapped data key.
pub(crate) fn sealed_fields(key: &WrappedKey) -> [(&'static str, Value); 4] {
    [
        ("sse_alg", Value::String(key.algorithm.clone())),
        ("sse_kek", Value::String(key.kek_id.clone())),
        ("sse_nonce", Value::Bytes(key.nonce.clone())),
        ("sse_key", Value::Bytes(key.ciphertext.clone())),
    ]
}

/// A record's wrapped data key: none when it carries no part of one, refused when it carries only some.
pub(crate) fn sealed(fields: &Fields) -> Result<Option<WrappedKey>> {
    let field = |name| {
        fields
            .get(name)
            .filter(|value| !matches!(value, Value::None))
    };
    match (
        field("sse_alg"),
        field("sse_kek"),
        field("sse_nonce"),
        field("sse_key"),
    ) {
        (None, None, None, None) => Ok(None),
        (
            Some(Value::String(algorithm)),
            Some(Value::String(kek_id)),
            Some(Value::Bytes(nonce)),
            Some(Value::Bytes(ciphertext)),
        ) => Ok(Some(WrappedKey {
            algorithm: algorithm.clone(),
            kek_id: kek_id.clone(),
            nonce: nonce.clone(),
            ciphertext: ciphertext.clone(),
        })),
        _ => Err(malformed("a wrapped data key")),
    }
}

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
    // A record a build before ADR-0008 wrote may still hold the object's bytes; it is converted at start and never
    // served from here.
    if fields.contains_key("inline") {
        return Err(malformed(
            "holds object bytes; the store is converted at start (ADR-0008)",
        ));
    }
    let content = match (fields.get("data"), fields.get("upload")) {
        (Some(Value::Uuid(id)), _) => Content::Data(*id),
        // The parts are read separately (`ObjectService::get`).
        (_, Some(Value::Uuid(upload))) => Content::Parts(Multipart {
            upload: *upload,
            parts: Vec::new(),
        }),
        _ => return Err(malformed("neither a data id nor an upload")),
    };
    let object = StoredObject {
        size: size(fields)?,
        etag: text(fields, "etag")?,
        modified: modified(fields)?,
        headers: read_strings(fields.get("headers"), "headers")?,
        metadata: read_strings(fields.get("metadata"), "metadata")?,
        tags: match fields.get("tags") {
            None | Some(Value::None) => BTreeMap::new(),
            tags => read_strings(tags, "tags")?,
        },
        checksums: read_strings(fields.get("checksums"), "checksums")?,
        content,
        version: match fields.get("version_id") {
            None | Some(Value::None) => None,
            Some(Value::String(id)) => Some(id.clone()),
            Some(_) => return Err(malformed("version_id")),
        },
        replication: match fields.get("replication_status") {
            None | Some(Value::None) => None,
            Some(Value::String(status)) => Some(status.clone()),
            Some(_) => return Err(malformed("replication_status")),
        },
        encryption: sealed(fields)?,
    };
    Ok((incarnation(fields)?, object))
}

/// A delete marker's incarnation and when it was written.
pub(crate) fn read_marker(value: &Value) -> Result<([u8; 16], tessari_s3_types::Timestamp)> {
    let fields = fields_of(value)?;
    Ok((incarnation(fields)?, modified(fields)?))
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
            content_type: read_strings(fields.get("headers"), "headers")?.remove("content-type"),
        },
    })
}

#[cfg(test)]
#[path = "entity_tests.rs"]
mod tests;
