//! Reading TessariDB answers: the first record of a read, and a bucket's incarnation out of it.

use tessari_s3_infrastructure::tessaridb::{Answer, Value};

use crate::{Error, Result};

/// The first record of a records answer.
pub(crate) fn first_record(answer: Option<Answer>) -> Result<Option<Value>> {
    match answer {
        Some(Answer::Records { records, .. }) => {
            Ok(records.into_iter().next().map(|(_, value)| value))
        }
        Some(Answer::Value {
            value: Value::None, ..
        })
        | None => Ok(None),
        Some(Answer::Value { value, .. }) => Ok(Some(value)),
        Some(_) => Err(Error::Malformed {
            record: "object",
            reason: "unexpected answer kind",
        }),
    }
}

pub(crate) fn incarnation_of(value: Option<Value>) -> Result<Option<[u8; 16]>> {
    match value {
        None => Ok(None),
        Some(Value::Object(fields)) => match fields.get("incarnation") {
            Some(Value::Uuid(bytes)) => Ok(Some(*bytes)),
            _ => Err(Error::Malformed {
                record: "bucket",
                reason: "incarnation",
            }),
        },
        Some(_) => Err(Error::Malformed {
            record: "bucket",
            reason: "not an object",
        }),
    }
}
