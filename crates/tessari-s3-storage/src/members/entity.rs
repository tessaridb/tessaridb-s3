//! An `s3_members` record exactly as stored.

use std::collections::BTreeMap;

use tessari_s3_infrastructure::tessaridb::Value;

use crate::{Error, Result};

/// The stored fields of one member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MemberEntity {
    /// The node id, also the record id.
    pub(crate) node: String,
    /// `approved` or `removed`.
    pub(crate) state: String,
    /// The pinned or bound certificate's SHA-256, hex.
    pub(crate) fingerprint: Option<String>,
    /// Whether an unused join token waits (its digest is never read back out).
    pub(crate) token_waits: bool,
    /// When that token lapses: seconds and nanoseconds since the epoch.
    pub(crate) token_expires: Option<(i64, u32)>,
    /// The operator's reason for the last change.
    pub(crate) reason: String,
    /// Who made it.
    pub(crate) by: String,
    /// When: seconds and nanoseconds since the epoch.
    pub(crate) at: (i64, u32),
}

fn malformed(reason: &'static str) -> Error {
    Error::Malformed {
        record: "member",
        reason,
    }
}

fn text(fields: &BTreeMap<String, Value>, field: &'static str) -> Result<String> {
    match fields.get(field) {
        Some(Value::String(text)) => Ok(text.clone()),
        _ => Err(malformed(field)),
    }
}

fn optional_text(fields: &BTreeMap<String, Value>, field: &'static str) -> Result<Option<String>> {
    match fields.get(field) {
        None | Some(Value::None) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        _ => Err(malformed(field)),
    }
}

fn instant(value: Option<&Value>, field: &'static str) -> Result<Option<(i64, u32)>> {
    match value {
        None | Some(Value::None) => Ok(None),
        Some(Value::Datetime { seconds, nanos }) => Ok(Some((*seconds, *nanos))),
        _ => Err(malformed(field)),
    }
}

impl MemberEntity {
    /// Reads a record's value.
    ///
    /// # Errors
    /// [`Error::Malformed`] when a field is missing or of another type.
    pub(crate) fn from_value(value: &Value) -> Result<Self> {
        let Value::Object(fields) = value else {
            return Err(malformed("not an object"));
        };
        Ok(Self {
            node: text(fields, "node")?,
            state: text(fields, "state")?,
            fingerprint: optional_text(fields, "fingerprint")?,
            token_waits: optional_text(fields, "token_digest")?.is_some(),
            token_expires: instant(fields.get("token_expires"), "token_expires")?,
            reason: text(fields, "reason")?,
            by: text(fields, "actor")?,
            at: instant(fields.get("at"), "at")?.ok_or_else(|| malformed("at"))?,
        })
    }
}
