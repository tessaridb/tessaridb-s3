//! Opening the encryption keys at start, for a node given `TESSARIDB_S3_KEYS_PASSPHRASE`. A node that cannot open
//! them still starts — plain objects are served, encrypted ones refused — and says why without the passphrase.

use tessari_s3_storage::Storage;
use tessari_s3_storage::keys::Passphrase;
use zeroize::Zeroizing;

/// Unseals the keys with `passphrase`, logging the outcome.
pub(crate) async fn unseal_at_start(storage: &Storage, passphrase: &str) {
    let checked = Passphrase::new(Zeroizing::new(passphrase.to_owned()));
    let opened = match checked {
        Ok(passphrase) => storage.keys().unseal(&passphrase).await,
        Err(error) => Err(error),
    };
    match opened {
        Ok(state) => {
            tracing::info!(state = ?state, "encryption keys unsealed from the environment")
        }
        Err(error) => tracing::error!(
            error = %error,
            "encryption keys not unsealed from TESSARIDB_S3_KEYS_PASSPHRASE; this node serves sealed"
        ),
    }
}
