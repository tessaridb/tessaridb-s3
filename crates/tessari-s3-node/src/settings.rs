//! The configuration this node runs on: the environment's bootstrap part reaches the metadata store, and the
//! settings stored there — written from the environment the first time a node starts with them — are read before
//! the rest of the configuration.

use tessari_s3_infrastructure::tessaridb::MetaPool;
use tessari_s3_infrastructure::{Bootstrap, S3Config};
use tessari_s3_storage::keys::Passphrase;
use tessari_s3_storage::settings::SettingsService;
use zeroize::Zeroizing;

/// The configuration from `env` and the settings the metadata store holds for this node's region.
///
/// A keys passphrase that is set must be a usable one: it opens the stored secrets, and a node that cannot open them
/// does not start on whatever its environment happens to hold instead.
///
/// # Errors
/// A variable missing or invalid, by name; the metadata store's refusal; a fixed setting the environment
/// contradicts, by name.
pub(crate) async fn configure(env: &impl Fn(&str) -> Option<String>) -> anyhow::Result<S3Config> {
    let bootstrap = Bootstrap::from_lookup(env)?;
    let passphrase = match &bootstrap.keys_passphrase {
        Some(text) => Some(Passphrase::new(Zeroizing::new(text.expose().to_owned()))?),
        None => None,
    };
    let settings = SettingsService::new(MetaPool::new(bootstrap.meta)?);
    let resolved = settings
        .resolve(&bootstrap.region, env, passphrase.as_ref())
        .await?;
    // Checked before anything is kept: a value this node refuses is never written for the others to start on.
    let config = S3Config::from_lookup(|key| resolved.lookup(env, key))?;
    settings
        .keep(&bootstrap.region, &resolved, passphrase.as_ref())
        .await?;
    Ok(config)
}
