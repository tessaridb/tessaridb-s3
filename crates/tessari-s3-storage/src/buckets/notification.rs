//! A bucket's notification configuration as the `notify` field of its record holds it: `{ rules: [ … ] }`, one object
//! per rule. The field's presence is what a write reads to decide whether its change leaves an event.

use std::collections::BTreeMap;

use tessari_s3_core::notification::Rule;
use tessari_s3_infrastructure::tessaridb::Value;

use crate::{Error, Result};

const fn malformed(reason: &'static str) -> Error {
    Error::Malformed {
        record: "bucket",
        reason,
    }
}

/// The stored form of `rules`.
pub(crate) fn to_value(rules: &[Rule]) -> Value {
    let rules = rules
        .iter()
        .map(|rule| {
            let mut fields = BTreeMap::new();
            fields.insert("id".to_owned(), Value::String(rule.id.clone()));
            fields.insert("target".to_owned(), Value::String(rule.target.clone()));
            fields.insert(
                "events".to_owned(),
                Value::Array(rule.events.iter().cloned().map(Value::String).collect()),
            );
            fields.insert("prefix".to_owned(), Value::String(rule.prefix.clone()));
            fields.insert("suffix".to_owned(), Value::String(rule.suffix.clone()));
            Value::Object(fields)
        })
        .collect();
    Value::Object(BTreeMap::from([("rules".to_owned(), Value::Array(rules))]))
}

/// The rules a stored `notify` field holds.
pub(crate) fn from_value(notify: &Value) -> Result<Vec<Rule>> {
    let Value::Object(configuration) = notify else {
        return Err(malformed("notify is not an object"));
    };
    let Some(Value::Array(rules)) = configuration.get("rules") else {
        return Err(malformed("notify holds no rules"));
    };
    rules
        .iter()
        .map(|rule| {
            let Value::Object(fields) = rule else {
                return Err(malformed("a notification rule is not an object"));
            };
            let text = |field: &'static str| match fields.get(field) {
                Some(Value::String(text)) => Ok(text.clone()),
                _ => Err(malformed("a notification rule's text field")),
            };
            let events = match fields.get("events") {
                Some(Value::Array(events)) => events
                    .iter()
                    .map(|event| match event {
                        Value::String(event) => Ok(event.clone()),
                        _ => Err(malformed("a notification rule's event")),
                    })
                    .collect::<Result<Vec<_>>>()?,
                _ => return Err(malformed("a notification rule's events")),
            };
            Ok(Rule {
                id: text("id")?,
                target: text("target")?,
                events,
                prefix: text("prefix")?,
                suffix: text("suffix")?,
            })
        })
        .collect()
}
