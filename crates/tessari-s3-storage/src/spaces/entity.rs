//! A `spaces` record exactly as stored.

use tessari_s3_infrastructure::tessaridb::Value;

use crate::{Error, Result};

/// The stored fields of one space.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpaceEntity {
    /// The space's name, also its record id.
    pub name: String,
    /// Creation time: seconds and nanoseconds since the epoch.
    pub created: (i64, u32),
}

impl SpaceEntity {
    /// Reads a record's value.
    ///
    /// # Errors
    /// [`Error::Malformed`] when a field is missing or of another type.
    pub fn from_value(value: &Value) -> Result<Self> {
        let malformed = |reason| Error::Malformed {
            record: "space",
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
        Ok(Self { name, created })
    }
}
