//! A bucket's lifecycle rules as the `lifecycle` field of its record holds them: an array of one object per rule.

use std::collections::BTreeMap;

use tessari_s3_core::lifecycle::{Expiration, Noncurrent, Rule};
use tessari_s3_infrastructure::tessaridb::{Number, Value};
use tessari_s3_types::Timestamp;

use crate::{Error, Result};

const fn malformed(reason: &'static str) -> Error {
    Error::Malformed {
        record: "bucket",
        reason,
    }
}

fn whole(value: Option<&Value>, reason: &'static str) -> Result<Option<u32>> {
    match value {
        None | Some(Value::None) => Ok(None),
        Some(Value::Number(Number::Integer(days))) => u32::try_from(*days)
            .map(Some)
            .map_err(|_| malformed(reason)),
        Some(_) => Err(malformed(reason)),
    }
}

/// The stored form of `rules`.
pub(crate) fn to_value(rules: &[Rule]) -> Value {
    Value::Array(
        rules
            .iter()
            .map(|rule| {
                let mut fields = BTreeMap::new();
                fields.insert("id".to_owned(), Value::String(rule.id.clone()));
                fields.insert("enabled".to_owned(), Value::Bool(rule.enabled));
                fields.insert("prefix".to_owned(), Value::String(rule.prefix.clone()));
                match &rule.expiration {
                    Some(Expiration::Days(days)) => {
                        fields.insert("expire_days".to_owned(), Value::from(i64::from(*days)));
                    }
                    Some(Expiration::Date(at)) => {
                        fields.insert(
                            "expire_at".to_owned(),
                            Value::Datetime {
                                seconds: at.seconds,
                                nanos: at.nanos,
                            },
                        );
                    }
                    None => {}
                }
                fields.insert(
                    "expired_marker".to_owned(),
                    Value::Bool(rule.expired_object_delete_marker),
                );
                if let Some(noncurrent) = &rule.noncurrent {
                    fields.insert(
                        "noncurrent_days".to_owned(),
                        Value::from(i64::from(noncurrent.days)),
                    );
                    if let Some(keep) = noncurrent.keep {
                        fields.insert("noncurrent_keep".to_owned(), Value::from(i64::from(keep)));
                    }
                }
                if let Some(days) = rule.abort_after_days {
                    fields.insert("abort_days".to_owned(), Value::from(i64::from(days)));
                }
                Value::Object(fields)
            })
            .collect(),
    )
}

/// The rules a stored `lifecycle` field holds.
pub(crate) fn from_values(rules: &[Value]) -> Result<Vec<Rule>> {
    rules
        .iter()
        .map(|rule| {
            let Value::Object(fields) = rule else {
                return Err(malformed("a lifecycle rule is not an object"));
            };
            let text = |field: &'static str| match fields.get(field) {
                Some(Value::String(text)) => Ok(text.clone()),
                _ => Err(malformed("a lifecycle rule's text field")),
            };
            let flag = |field: &'static str| matches!(fields.get(field), Some(Value::Bool(true)));
            let expiration = match (
                whole(fields.get("expire_days"), "expire_days")?,
                fields.get("expire_at"),
            ) {
                (Some(days), _) => Some(Expiration::Days(days)),
                (None, Some(Value::Datetime { seconds, nanos })) => {
                    Some(Expiration::Date(Timestamp {
                        seconds: *seconds,
                        nanos: *nanos,
                    }))
                }
                (None, None | Some(Value::None)) => None,
                (None, Some(_)) => return Err(malformed("expire_at")),
            };
            let noncurrent = whole(fields.get("noncurrent_days"), "noncurrent_days")?
                .map(|days| {
                    Ok::<_, Error>(Noncurrent {
                        days,
                        keep: whole(fields.get("noncurrent_keep"), "noncurrent_keep")?,
                    })
                })
                .transpose()?;
            Ok(Rule {
                id: text("id")?,
                enabled: flag("enabled"),
                prefix: text("prefix")?,
                expiration,
                expired_object_delete_marker: flag("expired_marker"),
                noncurrent,
                abort_after_days: whole(fields.get("abort_days"), "abort_days")?,
            })
        })
        .collect()
}
