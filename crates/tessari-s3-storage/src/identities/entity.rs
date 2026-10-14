//! The stored fields of the identities domain's records.

use tessari_s3_infrastructure::tessaridb::Value;

use crate::{Error, Result};

/// The stored fields of one in-flight login, found by its `state`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginEntity {
    /// The nonce the ID token must carry.
    pub nonce: String,
    /// The PKCE verifier.
    pub verifier: String,
}

/// The stored fields of one identity link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkEntity {
    /// The provider's issuer URL.
    pub issuer: String,
    /// The provider's subject.
    pub subject: String,
    /// The S3 user the identity signs in as.
    pub user_name: String,
}

/// The `after` of a `console_logouts` record: when its user's console sessions were last ended.
///
/// # Errors
/// [`Error::Malformed`] when the field is missing or not a whole number.
pub fn logout_after(value: &Value) -> Result<i64> {
    match value {
        Value::Object(fields) => match fields.get("after") {
            Some(Value::Number(tessari_s3_infrastructure::tessaridb::Number::Integer(at))) => {
                Ok(*at)
            }
            _ => Err(Error::Malformed {
                record: "console logout",
                reason: "after",
            }),
        },
        _ => Err(Error::Malformed {
            record: "console logout",
            reason: "not an object",
        }),
    }
}

fn text(value: &Value, record: &'static str, field: &'static str) -> Result<String> {
    match value {
        Value::Object(fields) => match fields.get(field) {
            Some(Value::String(text)) => Ok(text.clone()),
            _ => Err(Error::Malformed {
                record,
                reason: field,
            }),
        },
        _ => Err(Error::Malformed {
            record,
            reason: "not an object",
        }),
    }
}

impl LoginEntity {
    /// Reads a record's value.
    ///
    /// # Errors
    /// [`Error::Malformed`] when a field is missing or of another type.
    pub fn from_value(value: &Value) -> Result<Self> {
        Ok(Self {
            nonce: text(value, "login", "nonce")?,
            verifier: text(value, "login", "verifier")?,
        })
    }
}

impl LinkEntity {
    /// Reads a record's value.
    ///
    /// # Errors
    /// [`Error::Malformed`] when a field is missing or of another type.
    pub fn from_value(value: &Value) -> Result<Self> {
        Ok(Self {
            issuer: text(value, "identity link", "issuer")?,
            subject: text(value, "identity link", "subject")?,
            user_name: text(value, "identity link", "user_name")?,
        })
    }
}
