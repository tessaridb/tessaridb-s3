//! A `region_failovers` record exactly as stored.

use std::collections::BTreeMap;

use tessari_s3_infrastructure::tessaridb::{Number, Value};

use crate::{Error, Result};

/// The stored fields of one region's failover record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RegionEntity {
    /// The region, also the record id.
    pub(crate) region: String,
    /// How many failovers and restores the region has had.
    pub(crate) epoch: i64,
    /// The region its writes go to while it is failed over; absent once restored.
    pub(crate) promoted: Option<String>,
    /// The operator's reason for the last change.
    pub(crate) reason: String,
    /// Who made it.
    pub(crate) by: String,
    /// When: seconds and nanoseconds since the epoch.
    pub(crate) at: (i64, u32),
}

fn malformed(reason: &'static str) -> Error {
    Error::Malformed {
        record: "region failover",
        reason,
    }
}

fn text(fields: &BTreeMap<String, Value>, field: &'static str) -> Result<String> {
    match fields.get(field) {
        Some(Value::String(text)) => Ok(text.clone()),
        _ => Err(malformed(field)),
    }
}

impl RegionEntity {
    /// Reads a record's value.
    ///
    /// # Errors
    /// [`Error::Malformed`] when a field is missing or of another type.
    pub(crate) fn from_value(value: &Value) -> Result<Self> {
        let Value::Object(fields) = value else {
            return Err(malformed("not an object"));
        };
        Ok(Self {
            region: text(fields, "region")?,
            epoch: match fields.get("epoch") {
                Some(Value::Number(Number::Integer(epoch))) => *epoch,
                _ => return Err(malformed("epoch")),
            },
            promoted: match fields.get("promoted") {
                None | Some(Value::None) => None,
                Some(Value::String(region)) => Some(region.clone()),
                _ => return Err(malformed("promoted")),
            },
            reason: text(fields, "reason")?,
            by: text(fields, "actor")?,
            at: match fields.get("at") {
                Some(Value::Datetime { seconds, nanos }) => (*seconds, *nanos),
                _ => return Err(malformed("at")),
            },
        })
    }
}
