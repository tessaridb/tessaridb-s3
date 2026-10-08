//! A `console_actions` message exactly as a topic read answers it: `{ position, value }`.

use tessari_s3_infrastructure::tessaridb::{Number, Value};

use crate::{Error, Result};

/// The stored fields of one action, with its position in the topic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActionEntity {
    pub(crate) position: i64,
    /// Seconds and nanoseconds since the epoch.
    pub(crate) at: (i64, u32),
    pub(crate) operator: String,
    pub(crate) operation: String,
    pub(crate) target: String,
    pub(crate) reason: Option<String>,
    pub(crate) outcome: String,
}

impl ActionEntity {
    /// Reads one message of a `READ FROM` answer.
    pub(crate) fn from_value(value: &Value) -> Result<Self> {
        let malformed = |reason| Error::Malformed {
            record: "console_actions",
            reason,
        };
        let Value::Object(message) = value else {
            return Err(malformed("not an object"));
        };
        let position = match message.get("position") {
            Some(Value::Number(Number::Integer(position))) => *position,
            _ => return Err(malformed("position")),
        };
        let Some(Value::Object(fields)) = message.get("value") else {
            return Err(malformed("value"));
        };
        let text = |field: &'static str| match fields.get(field) {
            Some(Value::String(text)) => Ok(text.clone()),
            _ => Err(malformed(field)),
        };
        let at = match fields.get("at") {
            Some(Value::Datetime { seconds, nanos }) => (*seconds, *nanos),
            _ => return Err(malformed("at")),
        };
        let reason = match fields.get("reason") {
            None | Some(Value::None) => None,
            Some(Value::String(reason)) => Some(reason.clone()),
            Some(_) => return Err(malformed("reason")),
        };
        Ok(Self {
            position,
            at,
            operator: text("operator")?,
            operation: text("operation")?,
            target: text("target")?,
            reason,
            outcome: text("outcome")?,
        })
    }
}
