//! The keys passphrase from the environment, for nodes that must restart unattended.

use tessari_s3_types::SecretKey;

const KEY: &str = "TESSARIDB_S3_KEYS_PASSPHRASE";

/// `TESSARIDB_S3_KEYS_PASSPHRASE`, or `None` when unset or empty — then the node starts sealed and an operator
/// unseals it. Checked where it is used, and never echoed.
pub(crate) fn keys_passphrase(get: &impl Fn(&str) -> Option<String>) -> Option<SecretKey> {
    get(KEY).filter(|text| !text.is_empty()).map(SecretKey::new)
}

#[cfg(test)]
#[path = "keys_config_tests.rs"]
mod tests;
