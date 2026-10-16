//! Creating a key in the keys vault: one connection opens it, stores the key under an id nothing holds yet, and
//! seals it again.

use tessari_s3_infrastructure::tessaridb::{MetaError, Value, VaultLink};
use zeroize::Zeroizing;

use super::{CREATE_FIRST, declared_on, ids, unseal};
use crate::keys::Passphrase;
use crate::{Error, Result};

/// Stores `key` as record `id` on one connection: unseal, create, seal. `CREATE` refuses an id that exists, so a key
/// is never overwritten — and losing one would make every object sealed under it unreadable.
pub(super) async fn create_once(
    link: &mut VaultLink,
    passphrase: &Passphrase,
    id: &str,
    key: &Zeroizing<[u8; 32]>,
) -> Result<()> {
    if !declared_on(link).await? {
        return Err(Error::KeysSealed);
    }
    unseal(link, passphrase).await?;
    let created = link
        .run(
            CREATE_FIRST,
            vec![
                ("id".to_owned(), Value::String(id.to_owned())),
                ("key".to_owned(), Value::Bytes(key.to_vec())),
            ],
        )
        .await;
    let answer = match created {
        Ok(_) => Ok(()),
        Err(refused @ MetaError::Refused { .. }) => {
            if ids(link).await?.iter().any(|held| held == id) {
                Err(Error::KeyExists { id: id.to_owned() })
            } else {
                Err(refused.into())
            }
        }
        Err(other) => Err(other.into()),
    };
    if let Err(error) = link.seal().await {
        tracing::warn!(error = %error, "the keys vault was not sealed after a key was created; it lapses on its own");
    }
    answer
}
