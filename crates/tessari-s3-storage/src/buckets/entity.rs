//! A `buckets` record exactly as stored.

use tessari_s3_infrastructure::tessaridb::{Number, Value};

use crate::{Error, Result};

/// The stored fields of one bucket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BucketEntity {
    /// The bucket's name, held as a field so it is read back typed rather than parsed out of a record id.
    pub name: String,
    /// Creation time: seconds and nanoseconds since the epoch.
    pub created: (i64, u32),
    /// The region it was created in.
    pub region: String,
    /// A fresh uuid per creation, so a bucket deleted and created again is not the same bucket.
    pub incarnation: [u8; 16],
    /// The space that owns the bucket.
    pub space_name: String,
    /// The user who created it; absent for the root credential's buckets.
    pub creator: Option<String>,
    /// The byte limit an operator set; absent for none.
    pub max_bytes: Option<i64>,
    /// The object limit an operator set; absent for none.
    pub max_objects: Option<i64>,
}

impl BucketEntity {
    /// Reads a record's value.
    ///
    /// # Errors
    /// [`Error::Malformed`] when a field is missing or of another type.
    pub fn from_value(value: &Value) -> Result<Self> {
        let malformed = |reason| Error::Malformed {
            record: "bucket",
            reason,
        };
        let Value::Object(fields) = value else {
            return Err(malformed("not an object"));
        };
        let name = match fields.get("name") {
            Some(Value::String(name)) => name.clone(),
            _ => return Err(malformed("name")),
        };
        let created = match fields.get("created") {
            Some(Value::Datetime { seconds, nanos }) => (*seconds, *nanos),
            _ => return Err(malformed("created")),
        };
        let region = match fields.get("region") {
            Some(Value::String(region)) => region.clone(),
            _ => return Err(malformed("region")),
        };
        let incarnation = match fields.get("incarnation") {
            Some(Value::Uuid(bytes)) => *bytes,
            _ => return Err(malformed("incarnation")),
        };
        let space_name = match fields.get("space_name") {
            Some(Value::String(space)) => space.clone(),
            _ => return Err(malformed("space_name")),
        };
        let creator = match fields.get("creator") {
            None | Some(Value::None) => None,
            Some(Value::String(creator)) => Some(creator.clone()),
            Some(_) => return Err(malformed("creator")),
        };
        let limit = |field: &'static str| match fields.get(field) {
            None | Some(Value::None) => Ok(None),
            Some(Value::Number(Number::Integer(limit))) => Ok(Some(*limit)),
            Some(_) => Err(malformed(field)),
        };
        Ok(Self {
            name,
            created,
            region,
            incarnation,
            space_name,
            creator,
            max_bytes: limit("max_bytes")?,
            max_objects: limit("max_objects")?,
        })
    }
}
