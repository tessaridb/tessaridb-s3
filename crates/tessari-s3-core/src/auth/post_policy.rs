//! The policy of a browser form upload (S3 ref 03 M7): a base64 JSON document — an expiration and a list of
//! conditions — signed with the SigV4 signing key, and checked against the form AFTER `${filename}` is expanded, with
//! every field but the exempt ones covered by a condition.

use base64::Engine as _;
use serde_json::Value;
use tessari_s3_types::{SecretKey, Timestamp};

use super::encode::hex_lower;
use super::error::{AuthError, AuthResult};
use super::signing::{ALGORITHM, CredentialScope, SigningKey, check_signature};
use super::time::AmzDateTime;
use super::verified::{PayloadHash, Verified};

/// The largest policy document read, encoded.
const MAX_ENCODED: usize = 20 * 1024;

/// Fields no condition need name.
fn exempt(name: &str) -> bool {
    matches!(name, "x-amz-signature" | "file" | "policy") || name.starts_with("x-ignore-")
}

/// One condition, its field name lowercased and without the `$`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Condition {
    Exact { field: String, value: String },
    StartsWith { field: String, prefix: String },
    Length { min: u64, max: u64 },
}

fn malformed(reason: &'static str) -> PostPolicyError {
    PostPolicyError::Malformed(reason)
}

fn field_name(text: &str) -> Result<String, PostPolicyError> {
    text.strip_prefix('$')
        .filter(|name| !name.is_empty())
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| malformed("a condition names its field as $name"))
}

fn bound(value: &Value) -> Result<u64, PostPolicyError> {
    match value {
        Value::Number(number) => number.as_u64(),
        Value::String(text) => text.parse().ok(),
        _ => None,
    }
    .ok_or_else(|| malformed("a content-length-range bound is a whole number"))
}

fn condition(value: &Value) -> Result<Condition, PostPolicyError> {
    match value {
        Value::Object(pair) if pair.len() == 1 => {
            let (field, value) = pair
                .iter()
                .next()
                .ok_or_else(|| malformed("an empty condition"))?;
            Ok(Condition::Exact {
                field: field.to_ascii_lowercase(),
                value: value
                    .as_str()
                    .ok_or_else(|| malformed("an exact condition's value is a string"))?
                    .to_owned(),
            })
        }
        Value::Array(parts) => match parts.as_slice() {
            [
                Value::String(kind),
                Value::String(field),
                Value::String(value),
            ] => {
                let field = field_name(field)?;
                match kind.to_ascii_lowercase().as_str() {
                    "eq" => Ok(Condition::Exact {
                        field,
                        value: value.clone(),
                    }),
                    "starts-with" => Ok(Condition::StartsWith {
                        field,
                        prefix: value.clone(),
                    }),
                    _ => Err(malformed(
                        "a condition is eq, starts-with or content-length-range",
                    )),
                }
            }
            [Value::String(kind), min, max] if kind == "content-length-range" => {
                let (min, max) = (bound(min)?, bound(max)?);
                if min > max {
                    return Err(malformed(
                        "a content-length-range's minimum exceeds its maximum",
                    ));
                }
                Ok(Condition::Length { min, max })
            }
            _ => Err(malformed(
                "a condition is eq, starts-with or content-length-range",
            )),
        },
        _ => Err(malformed("a condition is an object or a list")),
    }
}

/// Why a form's policy refuses it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PostPolicyError {
    /// The document is not one this server can hold to its word.
    #[error("the POST policy is malformed: {0}")]
    Malformed(&'static str),
    /// Its expiration has passed.
    #[error("Invalid according to Policy: Policy expired.")]
    Expired,
    /// A condition on this field does not hold.
    #[error("Invalid according to Policy: Policy Condition failed: {0}")]
    ConditionFailed(String),
    /// A form field no condition names.
    #[error("Invalid according to Policy: Extra input fields: {0}")]
    NotCovered(String),
}

/// A parsed POST policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostPolicy {
    expiration: i64,
    conditions: Vec<Condition>,
}

impl PostPolicy {
    /// The policy in its base64 form.
    ///
    /// # Errors
    /// [`PostPolicyError::Malformed`].
    pub fn parse(encoded: &str) -> Result<Self, PostPolicyError> {
        if encoded.len() > MAX_ENCODED {
            return Err(malformed("a POST policy is at most 20 KB"));
        }
        let json = base64::engine::general_purpose::STANDARD
            .decode(encoded.trim())
            .map_err(|_| malformed("the policy is not base64"))?;
        // The document's shape is what is being validated here, once: an untyped tree is the honest first step.
        let root: Value =
            serde_json::from_slice(&json).map_err(|_| malformed("the policy is not JSON"))?;
        let root = root
            .as_object()
            .ok_or_else(|| malformed("the policy is not a JSON object"))?;
        let expiration = root
            .get("expiration")
            .and_then(Value::as_str)
            .and_then(Timestamp::parse_iso8601)
            .ok_or_else(|| malformed("the policy needs an ISO-8601 expiration"))?
            .seconds;
        let conditions = root
            .get("conditions")
            .and_then(Value::as_array)
            .ok_or_else(|| malformed("the policy needs a conditions list"))?
            .iter()
            .map(condition)
            .collect::<Result<_, _>>()?;
        Ok(Self {
            expiration,
            conditions,
        })
    }

    /// Checks `fields` (names lowercased, `${filename}` already expanded) of a form posted to `bucket` at `now_secs`;
    /// answers the content-length range the policy sets, if it sets one.
    ///
    /// # Errors
    /// [`PostPolicyError`] naming the first condition or field that fails.
    pub fn check(
        &self,
        bucket: &str,
        fields: &[(String, String)],
        now_secs: i64,
    ) -> Result<Option<(u64, u64)>, PostPolicyError> {
        if now_secs >= self.expiration {
            return Err(PostPolicyError::Expired);
        }
        let value = |field: &str| {
            if field == "bucket" {
                return Some(bucket);
            }
            fields
                .iter()
                .find(|(name, _)| name == field)
                .map(|(_, value)| value.as_str())
        };
        let mut range = None;
        for condition in &self.conditions {
            let (field, holds) = match condition {
                Condition::Exact {
                    field,
                    value: wanted,
                } => (field, value(field) == Some(wanted.as_str())),
                Condition::StartsWith { field, prefix } => (
                    field,
                    value(field)
                        .map_or(prefix.is_empty(), |seen| seen.starts_with(prefix.as_str())),
                ),
                Condition::Length { min, max } => {
                    range = Some((*min, *max));
                    continue;
                }
            };
            if !holds {
                return Err(PostPolicyError::ConditionFailed(field.clone()));
            }
        }
        let named = |name: &str| {
            self.conditions.iter().any(|condition| match condition {
                Condition::Exact { field, .. } | Condition::StartsWith { field, .. } => {
                    field == name
                }
                Condition::Length { .. } => false,
            })
        };
        if let Some((name, _)) = fields
            .iter()
            .find(|(name, _)| !exempt(name) && !named(name))
        {
            return Err(PostPolicyError::NotCovered(name.clone()));
        }
        Ok(range)
    }
}

/// The signature a client puts in `x-amz-signature` for `encoded` under `secret`, `date` (`YYYYMMDD`) and `region`.
///
/// # Errors
/// As the signing key's derivation.
pub fn post_signature(
    secret: &SecretKey,
    date: &str,
    region: &str,
    encoded: &str,
) -> AuthResult<String> {
    let scope = CredentialScope::parse(date, region, "s3", "aws4_request")?;
    Ok(hex_lower(
        &SigningKey::derive(secret, &scope)?.sign(encoded.as_bytes())?,
    ))
}

/// Verifies a form's signature fields against `secret` and this server's `region`.
///
/// # Errors
/// The SigV4 refusals: malformed fields, another algorithm, scope or region, a signature that does not match.
pub fn verify_post(
    encoded: &str,
    algorithm: &str,
    credential: &str,
    date: &str,
    signature: &str,
    secret: &SecretKey,
    region: &str,
) -> AuthResult<Verified> {
    if algorithm != ALGORITHM {
        return Err(AuthError::Malformed {
            reason: "x-amz-algorithm must be AWS4-HMAC-SHA256",
        });
    }
    let mut parts = credential.splitn(5, '/');
    let (Some(access_key), Some(day), Some(scope_region), Some(service), Some(terminator)) = (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) else {
        return Err(AuthError::Malformed {
            reason: "x-amz-credential",
        });
    };
    let scope = CredentialScope::parse(day, scope_region, service, terminator)?;
    let datetime = AmzDateTime::parse(date)?;
    scope.check(&datetime, region)?;
    let signing_key = SigningKey::derive(secret, &scope)?;
    let computed = signing_key.sign(encoded.as_bytes())?;
    check_signature(&computed, signature)?;
    Ok(Verified {
        access_key: access_key.to_owned(),
        scope,
        datetime,
        signing_key,
        signature: computed,
        payload: PayloadHash::Unsigned,
    })
}
