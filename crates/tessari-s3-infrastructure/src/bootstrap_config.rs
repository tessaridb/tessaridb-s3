//! What a node reads from its environment before it can read its stored settings: the way to the metadata store,
//! the region it serves and the passphrase that opens the store's secrets.

use tessari_s3_constants::DEFAULT_REGION;
use tessari_s3_types::SecretKey;

use crate::keys_config::keys_passphrase;
use crate::meta_config::meta_settings;
use crate::tessaridb::{MetaSettings, regional_database};
use crate::{Error, Result};

/// The environment-only part of the configuration, read before the metadata store is.
pub struct Bootstrap {
    /// The metadata store's address, credential and tenancy.
    pub meta: MetaSettings,
    /// `TESSARIDB_S3_REGION` — the region this node serves, which names its stored settings.
    pub region: String,
    /// `TESSARIDB_S3_KEYS_PASSPHRASE`, when set — it opens the store's secrets.
    pub keys_passphrase: Option<SecretKey>,
}

impl Bootstrap {
    /// Reads through `lookup` with the same checks [`crate::S3Config::from_lookup`] applies.
    ///
    /// # Errors
    /// [`Error::MissingConfig`] or [`Error::InvalidConfig`] naming the variable.
    pub fn from_lookup(lookup: &impl Fn(&str) -> Option<String>) -> Result<Self> {
        let get = |key: &str| lookup(key).filter(|value| !value.trim().is_empty());
        let meta = meta_settings(&get)?;
        let region = region(&get, &meta.database)?;
        Ok(Self {
            meta,
            region,
            keys_passphrase: keys_passphrase(&get),
        })
    }
}

/// `TESSARIDB_S3_REGION`, defaulted, when it can name its metadata database beside `database`.
pub(crate) fn region(get: &impl Fn(&str) -> Option<String>, database: &str) -> Result<String> {
    let region = get("TESSARIDB_S3_REGION").unwrap_or_else(|| DEFAULT_REGION.to_owned());
    if regional_database(database, &region).is_none() {
        return Err(Error::InvalidConfig {
            key: "TESSARIDB_S3_REGION",
            reason: "not a region name (lowercase letters, digits and single hyphens, beginning with a letter) \
                     short enough to name its metadata database",
        });
    }
    Ok(region)
}

#[cfg(test)]
#[path = "bootstrap_config_tests.rs"]
mod tests;
