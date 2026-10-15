//! A bucket's replication configuration as the `replication` field of its record holds it: the role and one object
//! per rule.

use std::collections::BTreeMap;

use tessari_s3_core::replication::{Configuration, Rule};
use tessari_s3_infrastructure::tessaridb::{Number, Value};

use crate::{Error, Result};

const fn malformed(reason: &'static str) -> Error {
    Error::Malformed {
        record: "bucket",
        reason,
    }
}

/// The stored form of `configuration`.
pub(crate) fn to_value(configuration: &Configuration) -> Value {
    let rules = configuration
        .rules
        .iter()
        .map(|rule| {
            let mut fields = BTreeMap::new();
            fields.insert("id".to_owned(), Value::String(rule.id.clone()));
            fields.insert("enabled".to_owned(), Value::Bool(rule.enabled));
            fields.insert("priority".to_owned(), Value::from(i64::from(rule.priority)));
            fields.insert("prefix".to_owned(), Value::String(rule.prefix.clone()));
            fields.insert(
                "destination".to_owned(),
                Value::String(rule.destination.clone()),
            );
            fields.insert(
                "delete_markers".to_owned(),
                Value::Bool(rule.delete_markers),
            );
            Value::Object(fields)
        })
        .collect();
    let mut fields = BTreeMap::new();
    fields.insert("role".to_owned(), Value::String(configuration.role.clone()));
    fields.insert("rules".to_owned(), Value::Array(rules));
    Value::Object(fields)
}

fn text(fields: &BTreeMap<String, Value>, name: &'static str) -> Result<String> {
    match fields.get(name) {
        Some(Value::String(text)) => Ok(text.clone()),
        _ => Err(malformed(name)),
    }
}

fn flag(fields: &BTreeMap<String, Value>, name: &'static str) -> Result<bool> {
    match fields.get(name) {
        Some(Value::Bool(flag)) => Ok(*flag),
        _ => Err(malformed(name)),
    }
}

/// The configuration the stored `value` holds.
pub(crate) fn from_value(value: &Value) -> Result<Configuration> {
    let Value::Object(fields) = value else {
        return Err(malformed("replication"));
    };
    let Some(Value::Array(stored)) = fields.get("rules") else {
        return Err(malformed("replication rules"));
    };
    let rules = stored
        .iter()
        .map(|rule| {
            let Value::Object(rule) = rule else {
                return Err(malformed("replication rule"));
            };
            let priority = match rule.get("priority") {
                Some(Value::Number(Number::Integer(priority))) => {
                    u32::try_from(*priority).map_err(|_| malformed("priority"))?
                }
                _ => return Err(malformed("priority")),
            };
            Ok(Rule {
                id: text(rule, "id")?,
                enabled: flag(rule, "enabled")?,
                priority,
                prefix: text(rule, "prefix")?,
                destination: text(rule, "destination")?,
                delete_markers: flag(rule, "delete_markers")?,
            })
        })
        .collect::<Result<_>>()?;
    Ok(Configuration {
        role: text(fields, "role")?,
        rules,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_configuration_reads_back_as_it_was_stored() {
        let configuration = Configuration {
            role: "arn:aws:iam::000000000000:role/r".to_owned(),
            rules: vec![Rule {
                id: "a".to_owned(),
                enabled: true,
                priority: 7,
                prefix: "logs/".to_owned(),
                destination: "backup".to_owned(),
                delete_markers: true,
            }],
        };
        assert_eq!(
            from_value(&to_value(&configuration)).expect("reads"),
            configuration
        );
        assert!(from_value(&Value::String("x".to_owned())).is_err());
    }
}
