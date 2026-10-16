//! A bucket's CORS rules as the `cors` field of its record holds them: an array of one object per rule.

use std::collections::BTreeMap;

use tessari_s3_core::cors::{Method, Rule};
use tessari_s3_infrastructure::tessaridb::{Number, Value};

use crate::{Error, Result};

const fn malformed(reason: &'static str) -> Error {
    Error::Malformed {
        record: "bucket",
        reason,
    }
}

fn texts(list: &[String]) -> Value {
    Value::Array(list.iter().cloned().map(Value::String).collect())
}

/// The stored form of `rules`.
pub(crate) fn to_value(rules: &[Rule]) -> Value {
    Value::Array(
        rules
            .iter()
            .map(|rule| {
                let mut fields = BTreeMap::new();
                if let Some(id) = &rule.id {
                    fields.insert("id".to_owned(), Value::String(id.clone()));
                }
                fields.insert("origins".to_owned(), texts(&rule.origins));
                fields.insert(
                    "methods".to_owned(),
                    Value::Array(
                        rule.methods
                            .iter()
                            .map(|method| Value::String(method.as_str().to_owned()))
                            .collect(),
                    ),
                );
                fields.insert("headers".to_owned(), texts(&rule.headers));
                fields.insert("expose".to_owned(), texts(&rule.expose));
                if let Some(seconds) = rule.max_age {
                    fields.insert("max_age".to_owned(), Value::from(i64::from(seconds)));
                }
                Value::Object(fields)
            })
            .collect(),
    )
}

fn strings(value: Option<&Value>) -> Result<Vec<String>> {
    let Some(Value::Array(items)) = value else {
        return Err(malformed("a CORS rule's list"));
    };
    items
        .iter()
        .map(|item| match item {
            Value::String(text) => Ok(text.clone()),
            _ => Err(malformed("a CORS rule's list item")),
        })
        .collect()
}

/// The rules a stored `cors` field holds.
pub(crate) fn from_values(rules: &[Value]) -> Result<Vec<Rule>> {
    rules
        .iter()
        .map(|rule| {
            let Value::Object(fields) = rule else {
                return Err(malformed("a CORS rule is not an object"));
            };
            let id = match fields.get("id") {
                None | Some(Value::None) => None,
                Some(Value::String(id)) => Some(id.clone()),
                Some(_) => return Err(malformed("a CORS rule's id")),
            };
            let methods = strings(fields.get("methods"))?
                .iter()
                .map(|name| Method::parse(name).ok_or_else(|| malformed("a CORS method")))
                .collect::<Result<_>>()?;
            let max_age = match fields.get("max_age") {
                None | Some(Value::None) => None,
                Some(Value::Number(Number::Integer(seconds))) => {
                    Some(u32::try_from(*seconds).map_err(|_| malformed("a CORS rule's max_age"))?)
                }
                Some(_) => return Err(malformed("a CORS rule's max_age")),
            };
            Ok(Rule {
                id,
                origins: strings(fields.get("origins"))?,
                methods,
                headers: strings(fields.get("headers"))?,
                expose: strings(fields.get("expose"))?,
                max_age,
            })
        })
        .collect()
}
