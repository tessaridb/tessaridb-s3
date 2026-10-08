//! A measurement exactly as TessariDB holds it: the grouped read's rows, and the `bucket_usage:1` record.

use tessari_s3_infrastructure::tessaridb::{Number, Value};

use crate::{Error, Result};

const fn malformed(reason: &'static str) -> Error {
    Error::Malformed {
        record: "bucket_usage",
        reason,
    }
}

/// One bucket's row of a measurement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BucketUsageEntity {
    pub(crate) bucket: String,
    pub(crate) objects: i64,
    pub(crate) bytes: i64,
    pub(crate) inline_bytes: i64,
    pub(crate) raw_bytes: i64,
}

impl BucketUsageEntity {
    /// Reads one element of the stored record's `buckets`. A record written before the occupancy figures existed has
    /// none; they read as zero until the next pass replaces it.
    pub(crate) fn from_value(value: &Value, bucket_field: &str) -> Result<Self> {
        let Value::Object(fields) = value else {
            return Err(malformed("not an object"));
        };
        let integer = |field: &'static str| match fields.get(field) {
            Some(Value::Number(Number::Integer(number))) => Ok(*number),
            _ => Err(malformed(field)),
        };
        let later = |field: &'static str| match fields.get(field) {
            None => Ok(0),
            Some(_) => integer(field),
        };
        let bucket = match fields.get(bucket_field) {
            Some(Value::String(bucket)) => bucket.clone(),
            _ => return Err(malformed("bucket")),
        };
        Ok(Self {
            bucket,
            objects: integer("objects")?,
            bytes: integer("bytes")?,
            inline_bytes: later("inline_bytes")?,
            raw_bytes: later("raw_bytes")?,
        })
    }

    /// The element of the stored record's `buckets` array.
    pub(crate) fn to_value(&self) -> Value {
        Value::Object(
            [
                ("bucket".to_owned(), Value::String(self.bucket.clone())),
                (
                    "objects".to_owned(),
                    Value::Number(Number::Integer(self.objects)),
                ),
                (
                    "bytes".to_owned(),
                    Value::Number(Number::Integer(self.bytes)),
                ),
                (
                    "inline_bytes".to_owned(),
                    Value::Number(Number::Integer(self.inline_bytes)),
                ),
                (
                    "raw_bytes".to_owned(),
                    Value::Number(Number::Integer(self.raw_bytes)),
                ),
            ]
            .into_iter()
            .collect(),
        )
    }
}

/// The stored measurement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UsageEntity {
    /// Seconds and nanoseconds since the epoch, by the metadata store's clock.
    pub(crate) taken: (i64, u32),
    pub(crate) buckets: Vec<BucketUsageEntity>,
}

impl UsageEntity {
    /// Reads the `bucket_usage:1` record.
    pub(crate) fn from_value(value: &Value) -> Result<Self> {
        let Value::Object(fields) = value else {
            return Err(malformed("not an object"));
        };
        let taken = match fields.get("taken") {
            Some(Value::Datetime { seconds, nanos }) => (*seconds, *nanos),
            _ => return Err(malformed("taken")),
        };
        let buckets = match fields.get("buckets") {
            Some(Value::Array(items)) => items
                .iter()
                .map(|item| BucketUsageEntity::from_value(item, "bucket"))
                .collect::<Result<Vec<_>>>()?,
            _ => return Err(malformed("buckets")),
        };
        Ok(Self { taken, buckets })
    }
}
