//! Condition operators (IAM condition operators page, retrieved 2026-10-17): a key absent from the request fails a
//! positive operator and satisfies a negated one; `…IfExists` is satisfied by an absent key; `Null` tests presence.
//! Several values under one key are alternatives; every key and every operator of a statement must hold.

use std::net::IpAddr;

use tessari_s3_types::Timestamp;

use super::context::{Context, Key, Kind, Observed, Requester};
use super::wildcard;

/// How the values compare with the request's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Test {
    Equals,
    EqualsIgnoreCase,
    Like,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
    Flag,
    InRange,
    Null,
}

/// One operator over one key, with the values it was given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Condition {
    test: Test,
    negated: bool,
    if_exists: bool,
    key: Key,
    values: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Value {
    Text(String),
    Number(i64),
    Flag(bool),
    Range(IpAddr, u8),
}

/// The operator `name` names: its test, whether it is negated, and the kind of key it compares.
fn operator(name: &str) -> Option<(Test, bool, Option<Kind>)> {
    let comparing = |kind| {
        move |rest: &str| -> Option<(Test, bool, Option<Kind>)> {
            Some(match rest {
                "Equals" => (Test::Equals, false, Some(kind)),
                "NotEquals" => (Test::Equals, true, Some(kind)),
                "LessThan" => (Test::Less, false, Some(kind)),
                "LessThanEquals" => (Test::LessOrEqual, false, Some(kind)),
                "GreaterThan" => (Test::Greater, false, Some(kind)),
                "GreaterThanEquals" => (Test::GreaterOrEqual, false, Some(kind)),
                _ => return None,
            })
        }
    };
    if let Some(rest) = name.strip_prefix("Numeric") {
        return comparing(Kind::Number)(rest);
    }
    if let Some(rest) = name.strip_prefix("Date") {
        return comparing(Kind::Instant)(rest);
    }
    Some(match name {
        "StringEquals" => (Test::Equals, false, Some(Kind::Text)),
        "StringNotEquals" => (Test::Equals, true, Some(Kind::Text)),
        "StringEqualsIgnoreCase" => (Test::EqualsIgnoreCase, false, Some(Kind::Text)),
        "StringNotEqualsIgnoreCase" => (Test::EqualsIgnoreCase, true, Some(Kind::Text)),
        "StringLike" => (Test::Like, false, Some(Kind::Text)),
        "StringNotLike" => (Test::Like, true, Some(Kind::Text)),
        "Bool" => (Test::Flag, false, Some(Kind::Flag)),
        "IpAddress" => (Test::InRange, false, Some(Kind::Address)),
        "NotIpAddress" => (Test::InRange, true, Some(Kind::Address)),
        "Null" => (Test::Null, false, None),
        _ => return None,
    })
}

impl Condition {
    /// The condition `operator_name` over `key_name` with `values` (each a policy's text); the message on refusal
    /// names what is wrong.
    pub(super) fn parse(
        operator_name: &str,
        key_name: &str,
        values: &[String],
    ) -> Result<Self, String> {
        if operator_name.contains(':') {
            return Err(format!(
                "the set operator in {operator_name} is not supported"
            ));
        }
        let (base, if_exists) = match operator_name.strip_suffix("IfExists") {
            Some(base) if base != "Null" => (base, true),
            _ => (operator_name, false),
        };
        let (test, negated, wanted) = operator(base)
            .ok_or_else(|| format!("the condition operator {operator_name} is not supported"))?;
        let key = Key::parse(key_name)
            .ok_or_else(|| format!("the condition key {key_name} is not supported"))?;
        let compatible = match (wanted, key.kind()) {
            (None | Some(Kind::Text), _) => true,
            (Some(Kind::Instant), Kind::Instant)
            | (Some(Kind::Number), Kind::Number | Kind::Instant) => true,
            (Some(wanted), kind) => wanted == kind,
        };
        if !compatible {
            return Err(format!("{operator_name} cannot test {key_name}"));
        }
        if values.is_empty() {
            return Err(format!("{operator_name} on {key_name} lists no value"));
        }
        let values = values
            .iter()
            .map(|text| {
                value(test, wanted, text)
                    .ok_or_else(|| format!("{text} is not a valid value for {operator_name}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            test,
            negated,
            if_exists,
            key,
            values,
        })
    }

    /// Whether the condition holds for `requester` in `context`.
    pub(super) fn holds(&self, requester: Requester<'_>, context: &Context) -> bool {
        let observed = self.key.observe(requester, context);
        if self.test == Test::Null {
            return self
                .values
                .iter()
                .any(|value| *value == Value::Flag(observed.is_none()));
        }
        let Some(observed) = observed else {
            return self.if_exists || self.negated;
        };
        let any = self
            .values
            .iter()
            .any(|value| compare(self.test, &observed, value));
        any != self.negated
    }
}

/// A policy's text read as the operator compares it.
fn value(test: Test, kind: Option<Kind>, text: &str) -> Option<Value> {
    match (test, kind) {
        (Test::Null | Test::Flag, _) => match text.to_ascii_lowercase().as_str() {
            "true" => Some(Value::Flag(true)),
            "false" => Some(Value::Flag(false)),
            _ => None,
        },
        (Test::InRange, _) => range(text),
        (_, Some(Kind::Number)) => number(text).map(Value::Number),
        (_, Some(Kind::Instant)) => instant(text).map(Value::Number),
        _ => Some(Value::Text(text.to_owned())),
    }
}

fn number(text: &str) -> Option<i64> {
    let digits = text.strip_prefix('-').unwrap_or(text);
    (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())).then(|| text.parse().ok())?
}

/// An instant as a policy writes it: epoch seconds, an ISO-8601 UTC instant, or a date (its midnight UTC).
fn instant(text: &str) -> Option<i64> {
    number(text)
        .or_else(|| Timestamp::parse_iso8601(text).map(|at| at.seconds))
        .or_else(|| Timestamp::parse_iso8601(&format!("{text}T00:00:00Z")).map(|at| at.seconds))
}

/// `a.b.c.d[/n]` or an IPv6 address with an optional prefix; no prefix is the single address.
fn range(text: &str) -> Option<Value> {
    let (address, prefix) = match text.split_once('/') {
        Some((address, prefix)) => (address, Some(prefix)),
        None => (text, None),
    };
    let address: IpAddr = address.parse().ok()?;
    let width = if address.is_ipv4() { 32 } else { 128 };
    let prefix = match prefix {
        Some(prefix) if !prefix.is_empty() && prefix.bytes().all(|b| b.is_ascii_digit()) => {
            prefix.parse::<u8>().ok()?
        }
        Some(_) => return None,
        None => width,
    };
    (prefix <= width).then_some(Value::Range(address, prefix))
}

fn compare(test: Test, observed: &Observed<'_>, value: &Value) -> bool {
    match (observed, value) {
        (Observed::Text(seen), Value::Text(wanted)) => match test {
            Test::Equals => seen == wanted,
            Test::EqualsIgnoreCase => seen.to_lowercase() == wanted.to_lowercase(),
            Test::Like => wildcard::matches(wanted, seen),
            _ => false,
        },
        (Observed::Number(seen), Value::Number(wanted)) => match test {
            Test::Equals => seen == wanted,
            Test::Less => seen < wanted,
            Test::LessOrEqual => seen <= wanted,
            Test::Greater => seen > wanted,
            Test::GreaterOrEqual => seen >= wanted,
            _ => false,
        },
        (Observed::Flag(seen), Value::Flag(wanted)) => seen == wanted,
        (Observed::Address(seen), Value::Range(network, prefix)) => {
            contains(*network, *prefix, *seen)
        }
        // A string operator over a key of another kind compares its text form.
        (Observed::Number(seen), Value::Text(wanted)) => compare(
            test,
            &Observed::Text(seen.to_string().into()),
            &Value::Text(wanted.clone()),
        ),
        (Observed::Flag(seen), Value::Text(wanted)) => compare(
            test,
            &Observed::Text(seen.to_string().into()),
            &Value::Text(wanted.clone()),
        ),
        (Observed::Address(seen), Value::Text(wanted)) => compare(
            test,
            &Observed::Text(seen.to_string().into()),
            &Value::Text(wanted.clone()),
        ),
        _ => false,
    }
}

/// Whether `address` lies in `network/prefix`; addresses of different families never do.
fn contains(network: IpAddr, prefix: u8, address: IpAddr) -> bool {
    let (network, address, width) = match (network, address) {
        (IpAddr::V4(n), IpAddr::V4(a)) => {
            (u128::from(u32::from(n)), u128::from(u32::from(a)), 32_u32)
        }
        (IpAddr::V6(n), IpAddr::V6(a)) => (u128::from(n), u128::from(a), 128_u32),
        _ => return false,
    };
    let host_bits = width.saturating_sub(u32::from(prefix));
    let mask = u128::MAX.checked_shl(host_bits).unwrap_or(0);
    network & mask == address & mask
}
