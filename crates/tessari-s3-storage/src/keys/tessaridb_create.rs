//! Creating a key in the keys vault — a named key, or the next server key of a rotation: one connection opens it,
//! stores the key under an id nothing holds yet, and seals it again.

use tessari_s3_infrastructure::tessaridb::{MetaError, Value, VaultLink};
use zeroize::Zeroizing;

use super::{CREATE_FIRST, declared_on, ids, unseal};
use crate::keys::Passphrase;
use crate::keys::model::{SERVER_KEY_PREFIX, server_number};
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
    let answer = store(link, id, key).await;
    seal_after(link).await;
    answer
}

/// Stores `key` as the next server key on one connection: unseal, read the ids, create one past the highest, seal.
/// Two rotations at once number the same id, and `CREATE` lets only one of them have it.
pub(super) async fn rotate_once(
    link: &mut VaultLink,
    passphrase: &Passphrase,
    key: &Zeroizing<[u8; 32]>,
) -> Result<String> {
    if !declared_on(link).await? {
        return Err(Error::KeysSealed);
    }
    unseal(link, passphrase).await?;
    let answer = match ids(link).await {
        Ok(held) => {
            let next = held
                .iter()
                .filter_map(|id| server_number(id))
                .max()
                .unwrap_or(0)
                .checked_add(1)
                .ok_or(Error::Unsealable)?;
            let id = format!("{SERVER_KEY_PREFIX}{next}");
            store(link, &id, key).await.map(|()| id)
        }
        Err(error) => Err(error),
    };
    seal_after(link).await;
    answer
}

/// Creates record `id` holding `key` in the unsealed vault; an id that exists is [`Error::KeyExists`].
async fn store(link: &mut VaultLink, id: &str, key: &Zeroizing<[u8; 32]>) -> Result<()> {
    let created = link
        .run(
            CREATE_FIRST,
            vec![
                ("id".to_owned(), Value::String(id.to_owned())),
                ("key".to_owned(), Value::Bytes(key.to_vec())),
            ],
        )
        .await;
    match created {
        Ok(_) => Ok(()),
        Err(refused @ MetaError::Refused { .. }) => {
            if ids(link).await?.iter().any(|held| held == id) {
                Err(Error::KeyExists { id: id.to_owned() })
            } else {
                Err(refused.into())
            }
        }
        Err(other) => Err(other.into()),
    }
}

async fn seal_after(link: &mut VaultLink) {
    if let Err(error) = link.seal().await {
        tracing::warn!(error = %error, "the keys vault was not sealed after a key was created; it lapses on its own");
    }
}
