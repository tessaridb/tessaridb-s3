//! Reading the settings this node runs on: what the store holds, then the environment, and the store given what only
//! the environment held.

use std::collections::HashMap;

use tessari_s3_infrastructure::settings_keys::{Class, STORED};
use tessari_s3_infrastructure::settings_source::{Resolved, resolve};
use tessari_s3_infrastructure::tessaridb::MetaPool;

use super::model::{record_id, variable_of};
use super::repository::SettingsStore;
use super::tessaridb::TessariSettings;
use crate::Result;
use crate::keys::Passphrase;

/// The settings service.
#[derive(Clone)]
pub struct SettingsService {
    store: TessariSettings,
}

impl SettingsService {
    /// Settings kept in the global database `pool` selects.
    #[must_use]
    pub const fn new(pool: MetaPool) -> Self {
        Self {
            store: TessariSettings::new(pool),
        }
    }
}

impl SettingsService {
    /// The settings `region` runs on, resolved from the store and `env`; every setting whose environment value the
    /// store overrides is logged by name. What only the environment held is not stored here: [`Self::keep`] stores it
    /// once the configuration it belongs to has been checked, so a value the node refuses is never kept.
    ///
    /// Secrets are read from, and seeded into, the store only with `passphrase` — the keys passphrase from the
    /// environment, which opens the secrets vault.
    ///
    /// # Errors
    /// The store's refusal or a malformed record; [`crate::Error::FixedSetting`] when the environment contradicts a setting
    /// that may not change once stored.
    pub async fn resolve(
        &self,
        region: &str,
        env: &impl Fn(&str) -> Option<String>,
        passphrase: Option<&Passphrase>,
    ) -> Result<Resolved> {
        let mut held: HashMap<String, String> = HashMap::new();
        for (id, value) in self.store.settings().await? {
            if let Some(key) = variable_of(&id, region) {
                held.insert(key.to_owned(), value);
            }
        }
        if let Some(passphrase) = passphrase {
            for (id, value) in self.store.secrets(passphrase).await? {
                if let Some(key) = variable_of(&id, region) {
                    held.insert(key.to_owned(), value.to_string());
                }
            }
        }
        let resolved =
            resolve(env, &held, passphrase.is_some()).map_err(|refused| match refused {
                tessari_s3_infrastructure::Error::InvalidConfig { key, reason } => {
                    crate::Error::FixedSetting { key, reason }
                }
                _ => crate::Error::Malformed {
                    record: "setting",
                    reason: "the stored settings could not be resolved",
                },
            })?;
        for key in resolved.differing() {
            tracing::warn!(
                setting = key,
                "the environment names a value the metadata store overrides; the stored value is used"
            );
        }
        Ok(resolved)
    }

    /// Stores, for `region`, every setting the environment held and the store did not; a secret goes into the secrets
    /// vault with `passphrase`. One already stored by another node is left as it is.
    ///
    /// # Errors
    /// The store's refusal.
    pub async fn keep(
        &self,
        region: &str,
        resolved: &Resolved,
        passphrase: Option<&Passphrase>,
    ) -> Result<()> {
        for (key, value) in resolved.seed() {
            let Some(stored) = STORED.iter().find(|stored| stored.key == key) else {
                continue;
            };
            let id = record_id(stored, region);
            match (stored.class, passphrase) {
                (Class::Secret, Some(passphrase)) => {
                    self.store.seed_secret(passphrase, &id, value).await?
                }
                _ => self.store.seed_setting(&id, value).await?,
            }
            tracing::info!(
                setting = key,
                "a setting the environment held is now kept in the metadata store"
            );
        }
        Ok(())
    }
}
