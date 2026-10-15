//! Values the server computes itself and binds into statements rather than formatting them into the text.

use tessari_s3_infrastructure::tessaridb::{Number, Value};

/// A count for `LIMIT $n`; one past `i64` is clamped to it, which no read reaches.
pub(crate) fn count(n: usize) -> Value {
    Value::Number(Number::Integer(i64::try_from(n).unwrap_or(i64::MAX)))
}

/// A number of seconds, for `duration::from_secs($n)`; one past `i64` is clamped to it.
pub(crate) fn seconds(n: u64) -> Value {
    Value::Number(Number::Integer(i64::try_from(n).unwrap_or(i64::MAX)))
}
