//! The IAM half of the configuration: the root key access-key secrets are sealed under.

use tessari_s3_types::IamKey;

use crate::{Error, Result};

const KEY: &str = "TESSARIDB_S3_IAM_KEY";

/// `TESSARIDB_S3_IAM_KEY`: 64 hexadecimal digits, or unset — then no user access key can be issued or used, and the
/// root credential is the only one.
///
/// # Errors
/// [`crate::Error::InvalidConfig`] for a value that is not 64 hexadecimal digits; the value is never echoed.
pub(crate) fn iam_key(get: &impl Fn(&str) -> Option<String>) -> Result<Option<IamKey>> {
    let Some(text) = get(KEY).map(zeroize::Zeroizing::new) else {
        return Ok(None);
    };
    IamKey::from_hex(&text)
        .map(Some)
        .ok_or(Error::InvalidConfig {
            key: KEY,
            reason: "not 64 hexadecimal digits",
        })
}

#[cfg(test)]
#[path = "iam_config_tests.rs"]
mod tests;
