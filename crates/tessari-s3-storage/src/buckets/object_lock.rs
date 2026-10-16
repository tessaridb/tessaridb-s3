//! A bucket's Object Lock as it is stored: `object_lock` is present once the lock is on (an empty object when there is
//! no default retention) and carries the default's `mode` and `days` or `years`. The store's `object_lock_kept` event
//! keeps it present and the bucket's versioning enabled (ADR-0016).

use std::collections::BTreeMap;

use tessari_s3_core::object_lock::{Configuration, DefaultRetention, Mode, Period};
use tessari_s3_infrastructure::tessaridb::{Number, Value};

use crate::{Error, Result};

const fn malformed(reason: &'static str) -> Error {
    Error::Malformed {
        record: "bucket",
        reason,
    }
}

pub(crate) fn to_value(configuration: &Configuration) -> Value {
    let mut fields = BTreeMap::new();
    if let Some(default) = configuration.default {
        fields.insert(
            "mode".to_owned(),
            Value::String(default.mode.as_str().to_owned()),
        );
        let (name, count) = match default.period {
            Period::Days(days) => ("days", days),
            Period::Years(years) => ("years", years),
        };
        fields.insert(name.to_owned(), Value::from(i64::from(count)));
    }
    Value::Object(fields)
}

fn count(value: Option<&Value>) -> Result<Option<u16>> {
    match value {
        None | Some(Value::None) => Ok(None),
        Some(Value::Number(Number::Integer(count))) => u16::try_from(*count)
            .map(Some)
            .map_err(|_| malformed("a default retention period")),
        Some(_) => Err(malformed("a default retention period")),
    }
}

pub(crate) fn from_value(value: &Value) -> Result<Configuration> {
    let Value::Object(fields) = value else {
        return Err(malformed("object_lock"));
    };
    let Some(mode) = fields.get("mode") else {
        return Ok(Configuration::default());
    };
    let mode = match mode {
        Value::String(mode) => Mode::parse(mode).ok_or(malformed("a default retention mode"))?,
        _ => return Err(malformed("a default retention mode")),
    };
    let period = match (count(fields.get("days"))?, count(fields.get("years"))?) {
        (Some(days), None) => Period::Days(days),
        (None, Some(years)) => Period::Years(years),
        _ => return Err(malformed("a default retention period")),
    };
    let default =
        DefaultRetention::new(mode, period).ok_or(malformed("a default retention period"))?;
    Ok(Configuration {
        default: Some(default),
    })
}

#[cfg(test)]
mod tests {
    use tessari_s3_core::object_lock::{Configuration, DefaultRetention, Mode, Period};

    use super::{from_value, to_value};

    #[test]
    fn a_configuration_round_trips_through_its_stored_form() {
        let none = Configuration::default();
        assert_eq!(from_value(&to_value(&none)).expect("reads"), none);
        for period in [Period::Days(30), Period::Years(7)] {
            let with = Configuration {
                default: DefaultRetention::new(Mode::Compliance, period),
            };
            assert!(with.default.is_some());
            assert_eq!(from_value(&to_value(&with)).expect("reads"), with);
        }
    }
}
