//! A parts record as CompleteMultipartUpload reads it.

use tessari_s3_core::objects::multipart::PartRecord;
use tessari_s3_infrastructure::tessaridb::{Number, Value};

use super::repository::PartRow;
use super::tessaridb::{malformed, string_map};
use crate::Result;

/// `number, data, size, etag, checksums` of one part.
pub(super) fn complete_row(value: &Value) -> Result<PartRow> {
    let Value::Object(fields) = value else {
        return Err(malformed("not an object"));
    };
    let integer = |field: &'static str| match fields.get(field) {
        Some(Value::Number(Number::Integer(n))) => Ok(*n),
        _ => Err(malformed(field)),
    };
    Ok(PartRow {
        number: u16::try_from(integer("number")?).map_err(|_| malformed("number"))?,
        data: match fields.get("data") {
            Some(Value::Uuid(id)) => *id,
            _ => return Err(malformed("data")),
        },
        record: PartRecord {
            size: u64::try_from(integer("size")?).map_err(|_| malformed("size"))?,
            etag: match fields.get("etag") {
                Some(Value::String(etag)) => etag.clone(),
                _ => return Err(malformed("etag")),
            },
            checksums: string_map(fields.get("checksums"))?,
        },
    })
}
