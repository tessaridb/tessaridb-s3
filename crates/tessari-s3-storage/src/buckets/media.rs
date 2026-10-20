//! A bucket's media rules as the `media` field of its record holds them: an array of one object per rule, presets by
//! name. The field's presence also makes the bucket's changes recorded in `object_events` for its media worker.

use std::collections::BTreeMap;

use tessari_s3_core::media::{Output, Rule};
use tessari_s3_infrastructure::tessaridb::Value;

use crate::{Error, Result};

const fn malformed(reason: &'static str) -> Error {
    Error::Malformed {
        record: "bucket",
        reason,
    }
}

fn texts(values: &[String]) -> Value {
    Value::Array(values.iter().cloned().map(Value::String).collect())
}

/// The stored form of `rules`.
pub(crate) fn to_value(rules: &[Rule]) -> Value {
    Value::Array(
        rules
            .iter()
            .map(|rule| {
                let outputs: Vec<String> =
                    rule.outputs.iter().map(|output| output.name()).collect();
                Value::Object(BTreeMap::from([
                    ("id".to_owned(), Value::String(rule.id.clone())),
                    ("content_types".to_owned(), texts(&rule.content_types)),
                    ("prefix".to_owned(), Value::String(rule.prefix.clone())),
                    ("outputs".to_owned(), texts(&outputs)),
                ]))
            })
            .collect(),
    )
}

fn strings(value: Option<&Value>, reason: &'static str) -> Result<Vec<String>> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| match item {
                Value::String(text) => Ok(text.clone()),
                _ => Err(malformed(reason)),
            })
            .collect(),
        _ => Err(malformed(reason)),
    }
}

/// The rules a stored `media` field holds.
pub(crate) fn from_values(rules: &[Value]) -> Result<Vec<Rule>> {
    rules
        .iter()
        .map(|rule| {
            let Value::Object(fields) = rule else {
                return Err(malformed("a media rule is not an object"));
            };
            let text = |field: &'static str| match fields.get(field) {
                Some(Value::String(text)) => Ok(text.clone()),
                _ => Err(malformed("a media rule's text field")),
            };
            let outputs = strings(fields.get("outputs"), "a media rule's outputs")?
                .iter()
                .map(|name| Output::parse(name).map_err(|_| malformed("a media rule's output")))
                .collect::<Result<Vec<_>>>()?;
            Ok(Rule {
                id: text("id")?,
                content_types: strings(
                    fields.get("content_types"),
                    "a media rule's content types",
                )?,
                prefix: text("prefix")?,
                outputs,
            })
        })
        .collect()
}
