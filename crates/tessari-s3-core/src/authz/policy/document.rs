//! The bucket policy document: parsed once from its JSON text into a typed form, refusing — with the reason — every
//! shape this server would not evaluate exactly as written.

use std::fmt;

use serde_json::{Map, Value};
use tessari_s3_constants::access::BUCKET_POLICY_MAX_BYTES;

use super::super::model::SpaceName;
use super::actions::S3_ACTIONS;
use super::condition::Condition;
use super::wildcard;

/// Why a policy was refused, in words a caller can act on (answered as `MalformedPolicy`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyError(String);

impl fmt::Display for PolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PolicyError {}

fn refuse(message: impl Into<String>) -> PolicyError {
    PolicyError(message.into())
}

/// Whether a statement allows or denies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Grants what it matches.
    Allow,
    /// Refuses what it matches, whatever else allows it.
    Deny,
}

/// Who a statement is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Principals {
    /// `"*"`: every caller, anonymous included.
    Anyone,
    /// Named users of the bucket's space, or all of them.
    Named {
        whole_space: bool,
        users: Vec<String>,
    },
}

/// `Action`/`Resource` patterns, or their `Not…` complements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Patterns {
    pub(super) negated: bool,
    pub(super) list: Vec<String>,
}

/// One statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Statement {
    pub(super) sid: Option<String>,
    pub(super) effect: Effect,
    pub(super) principals: Principals,
    pub(super) actions: Patterns,
    pub(super) resources: Patterns,
    pub(super) conditions: Vec<Condition>,
}

/// A bucket's policy, typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BucketPolicy {
    pub(super) bucket: String,
    pub(super) space: SpaceName,
    pub(super) statements: Vec<Statement>,
}

impl BucketPolicy {
    /// The policy `text` for bucket `bucket` of `space`.
    ///
    /// # Errors
    /// [`PolicyError`] naming the first thing that is malformed or not evaluable here.
    pub fn parse(text: &str, bucket: &str, space: &SpaceName) -> Result<Self, PolicyError> {
        if text.len() > BUCKET_POLICY_MAX_BYTES {
            return Err(refuse(format!(
                "a bucket policy is at most {} KB",
                BUCKET_POLICY_MAX_BYTES / 1024
            )));
        }
        // An untyped tree is the honest first step: the document's shape is what is being validated, once.
        let root: Value =
            serde_json::from_str(text).map_err(|_| refuse("the policy is not JSON"))?;
        let root = root
            .as_object()
            .ok_or_else(|| refuse("the policy is not a JSON object"))?;
        for key in root.keys() {
            if !matches!(key.as_str(), "Version" | "Id" | "Statement") {
                return Err(refuse(format!("the policy key {key} is not supported")));
            }
        }
        match root.get("Version").and_then(Value::as_str) {
            Some("2012-10-17" | "2008-10-17") => {}
            _ => return Err(refuse("the policy needs a Version of 2012-10-17")),
        }
        let listed = match root.get("Statement") {
            Some(Value::Array(list)) => list.iter().collect::<Vec<_>>(),
            Some(single @ Value::Object(_)) => vec![single],
            _ => Vec::new(),
        };
        if listed.is_empty() {
            return Err(refuse("the policy has no statement"));
        }
        let statements = listed
            .into_iter()
            .map(|value| statement(value, bucket, space))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            bucket: bucket.to_owned(),
            space: space.clone(),
            statements,
        })
    }

    /// The statements, in the order written.
    #[must_use]
    pub fn statements(&self) -> &[Statement] {
        &self.statements
    }
}

fn statement(value: &Value, bucket: &str, space: &SpaceName) -> Result<Statement, PolicyError> {
    let fields = value
        .as_object()
        .ok_or_else(|| refuse("a statement is not a JSON object"))?;
    for key in fields.keys() {
        match key.as_str() {
            "Sid" | "Effect" | "Principal" | "Action" | "NotAction" | "Resource"
            | "NotResource" | "Condition" => {}
            "NotPrincipal" => {
                return Err(refuse(
                    "NotPrincipal is not supported; name the principals instead",
                ));
            }
            other => {
                return Err(refuse(format!(
                    "the statement key {other} is not supported"
                )));
            }
        }
    }
    if text_of(value).contains("${") {
        return Err(refuse("policy variables (${…}) are not supported"));
    }
    let sid = match fields.get("Sid") {
        None => None,
        Some(Value::String(sid)) => Some(sid.clone()),
        Some(_) => return Err(refuse("a Sid is a string")),
    };
    let effect = match fields.get("Effect").and_then(Value::as_str) {
        Some("Allow") => Effect::Allow,
        Some("Deny") => Effect::Deny,
        _ => return Err(refuse("a statement's Effect is Allow or Deny")),
    };
    let principals = principals(
        fields
            .get("Principal")
            .ok_or_else(|| refuse("a statement needs a Principal"))?,
        space,
    )?;
    let actions = patterns(fields, "Action", "NotAction")?;
    for action in &actions.list {
        known_action(action)?;
    }
    let resources = patterns(fields, "Resource", "NotResource")?;
    for resource in &resources.list {
        this_bucket(resource, bucket)?;
    }
    let conditions = match fields.get("Condition") {
        None => Vec::new(),
        Some(value) => conditions(value)?,
    };
    Ok(Statement {
        sid,
        effect,
        principals,
        actions,
        resources,
        conditions,
    })
}

fn text_of(value: &Value) -> String {
    value.to_string()
}

/// A string or a list of strings.
fn strings(value: &Value, what: &str) -> Result<Vec<String>, PolicyError> {
    let as_text = |value: &Value| match value {
        Value::String(text) => Ok(text.clone()),
        Value::Bool(flag) => Ok(flag.to_string()),
        Value::Number(number) => Ok(number.to_string()),
        _ => Err(refuse(format!(
            "{what} lists something that is not a string"
        ))),
    };
    match value {
        Value::Array(items) => items.iter().map(as_text).collect(),
        other => Ok(vec![as_text(other)?]),
    }
}

fn principals(value: &Value, space: &SpaceName) -> Result<Principals, PolicyError> {
    let named = match value {
        Value::String(star) if star == "*" => return Ok(Principals::Anyone),
        Value::Object(kinds) => {
            if let Some(kind) = kinds.keys().find(|kind| kind.as_str() != "AWS") {
                return Err(refuse(format!(
                    "the principal kind {kind} is not supported"
                )));
            }
            strings(
                kinds
                    .get("AWS")
                    .ok_or_else(|| refuse("a Principal object names AWS principals"))?,
                "Principal",
            )?
        }
        _ => return Err(refuse("a Principal is \"*\" or {\"AWS\": …}")),
    };
    if named.iter().any(|name| name == "*") {
        return Ok(Principals::Anyone);
    }
    let prefix = format!("arn:aws:iam::{}:", space.as_str());
    let (mut whole_space, mut users) = (false, Vec::new());
    for arn in named {
        match arn.strip_prefix(&prefix) {
            Some("root") => whole_space = true,
            Some(rest)
                if rest
                    .strip_prefix("user/")
                    .is_some_and(|name| !name.is_empty() && !name.contains(['*', '?'])) =>
            {
                users.push(rest.trim_start_matches("user/").to_owned());
            }
            _ => {
                return Err(refuse(format!(
                    "the principal {arn} is not a user or the root of this bucket's space"
                )));
            }
        }
    }
    Ok(Principals::Named { whole_space, users })
}

fn patterns(
    fields: &Map<String, Value>,
    plain: &str,
    negated: &str,
) -> Result<Patterns, PolicyError> {
    match (fields.get(plain), fields.get(negated)) {
        (Some(_), Some(_)) => Err(refuse(format!(
            "a statement has both {plain} and {negated}"
        ))),
        (Some(value), None) => Ok(Patterns {
            negated: false,
            list: strings(value, plain)?,
        }),
        (None, Some(value)) => Ok(Patterns {
            negated: true,
            list: strings(value, negated)?,
        }),
        (None, None) => Err(refuse(format!("a statement needs {plain} or {negated}"))),
    }
}

/// An action pattern that names, or by wildcard matches, at least one S3 action.
fn known_action(pattern: &str) -> Result<(), PolicyError> {
    let unknown = || refuse(format!("the action {pattern} is not an S3 action"));
    if pattern == "*" {
        return Ok(());
    }
    let (service, name) = pattern.split_once(':').ok_or_else(unknown)?;
    if !service.eq_ignore_ascii_case("s3") {
        return Err(unknown());
    }
    S3_ACTIONS
        .iter()
        .any(|action| wildcard::matches_ignoring_case(name, action))
        .then_some(())
        .ok_or_else(unknown)
}

/// A resource naming this bucket or keys within it.
fn this_bucket(resource: &str, bucket: &str) -> Result<(), PolicyError> {
    let arn = format!("arn:aws:s3:::{bucket}");
    let ok = resource == arn
        || resource
            .strip_prefix(&arn)
            .is_some_and(|rest| rest.starts_with('/'));
    ok.then_some(()).ok_or_else(|| {
        refuse(format!(
            "the resource {resource} is not this bucket or a key in it"
        ))
    })
}

fn conditions(value: &Value) -> Result<Vec<Condition>, PolicyError> {
    let operators = value
        .as_object()
        .ok_or_else(|| refuse("a Condition is a JSON object"))?;
    let mut parsed = Vec::new();
    for (operator, keys) in operators {
        let keys = keys
            .as_object()
            .ok_or_else(|| refuse(format!("the condition {operator} is not a JSON object")))?;
        for (key, values) in keys {
            let values = strings(values, operator)?;
            parsed.push(Condition::parse(operator, key, &values).map_err(refuse)?);
        }
    }
    Ok(parsed)
}
