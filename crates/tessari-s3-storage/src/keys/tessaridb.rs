//! The keys vault in TessariDB: `s3_keys` in the global database, declared with its OWN passphrase so neither the
//! store's passphrase nor any authority in the store opens it. One record per key-encryption key, its 32 bytes in a
//! `SECRET` field. Every read of it is a `REVEAL`, which the engine records before it answers.

use tessari_s3_constants::{KEYS_OPEN_ATTEMPTS, KEYS_OPEN_BACKOFF_MS};
use tessari_s3_infrastructure::tessaridb::{
    MetaError, MetaPool, RecordId, RecordRef, RefusalClass, SealState, Value, VaultLink,
};
use zeroize::Zeroizing;

use super::entity::StoredKey;
use super::model::{Passphrase, SERVER_KEY_PREFIX};
use super::repository::KeyVault;
use crate::answers::first_record;
use crate::{Error, Result};

/// The vault's name.
const VAULT: &str = "s3_keys";
/// Its fields; re-run on every open, so a node that stopped between declaring the vault and its field converges.
const FIELDS: &str = "DEFINE FIELD IF NOT EXISTS key ON s3_keys TYPE bytes SECRET;";
/// Times one open is tried when another node seals the vault between this one's unseal and its reads.
/// Most keys read in one open: one listing page.
const MAX_KEYS: u32 = 10_000;

/// The TessariDB keys vault.
#[derive(Clone)]
pub struct TessariKeys {
    pool: MetaPool,
}

impl TessariKeys {
    /// The vault in the database `pool` selects (the global one).
    #[must_use]
    pub const fn new(pool: MetaPool) -> Self {
        Self { pool }
    }
}

/// Whether the database `info` describes names the keys vault.
fn names_vault(info: Option<Value>) -> bool {
    let Some(Value::Object(mut info)) = info else {
        return false;
    };
    match info.remove("vaults") {
        Some(Value::Array(names) | Value::Set(names)) => names
            .iter()
            .any(|name| matches!(name, Value::String(name) if name == VAULT)),
        Some(Value::Object(names)) => names.contains_key(VAULT),
        _ => false,
    }
}

async fn declared_on(link: &mut VaultLink) -> Result<bool> {
    let mut answers = link.run("INFO FOR DATABASE;", Vec::new()).await?;
    Ok(names_vault(first_record(answers.pop())?))
}

/// Declares the vault. The passphrase is the one literal this domain writes into statement text, because the engine
/// takes an own passphrase no other way (dogfood DB-22); [`Passphrase`] admits nothing that needs escaping, and a
/// refusal — whose text may quote the statement — is never passed on.
async fn declare(link: &mut VaultLink, passphrase: &Passphrase) -> Result<()> {
    let statement = Zeroizing::new(format!(
        "DEFINE VAULT {VAULT} PASSPHRASE '{}';",
        passphrase.expose()
    ));
    match link.run(&statement, Vec::new()).await {
        Ok(_) => Ok(()),
        // Another node declared it first; the caller reads it back.
        Err(MetaError::Refused { .. }) if declared_on(link).await? => Ok(()),
        Err(MetaError::Refused { class, .. }) => Err(Error::Meta(MetaError::Refused {
            message: "the keys vault could not be declared".to_owned(),
            class,
        })),
        Err(other) => Err(other.into()),
    }
}

/// Unseals the vault on the node `link` reached, unless it is already open there.
async fn unseal(link: &mut VaultLink, passphrase: &Passphrase) -> Result<()> {
    if link.status().await?.state == SealState::Unsealed {
        return Ok(());
    }
    match link.unseal(passphrase.expose()).await {
        Ok(_) => Ok(()),
        Err(MetaError::Refused {
            class: Some(RefusalClass::Throttled),
            ..
        }) => Err(Error::PassphraseThrottled),
        // A wrong passphrase, or another node unsealing it in between: only the second leaves it open.
        Err(MetaError::Refused {
            class: Some(RefusalClass::Invalid),
            ..
        }) => {
            if link.status().await?.state == SealState::Unsealed {
                Ok(())
            } else {
                Err(Error::WrongPassphrase)
            }
        }
        Err(other) => Err(other.into()),
    }
}

/// The ids of the vault's records.
async fn ids(link: &mut VaultLink) -> Result<Vec<String>> {
    let script = format!("INFO FOR VAULT {VAULT} RECORDS LIMIT {MAX_KEYS};");
    let mut answers = link.run(&script, Vec::new()).await?;
    let Some(Value::Object(mut page)) = first_record(answers.pop())? else {
        return Err(malformed("not a listing"));
    };
    let Some(Value::Array(records)) = page.remove("records") else {
        return Err(malformed("a listing without records"));
    };
    records
        .into_iter()
        .map(|record| match record {
            Value::String(id)
            | Value::Record(RecordRef {
                id: RecordId::Text(id),
                ..
            }) => Ok(id),
            _ => Err(malformed("a record id that is not text")),
        })
        .collect()
}

/// Creates the first server key. `CREATE` refuses an id that exists, so two nodes initialising at once never
/// overwrite each other: the loser reads the winner's key.
const CREATE_FIRST: &str = "CREATE s3_keys:$id = { key: $key };";

/// Stores `first` as the first server key; a node that loses the race to create it reads the winner's instead.
async fn create_first(link: &mut VaultLink, first: &Zeroizing<[u8; 32]>) -> Result<()> {
    let created = link
        .run(
            CREATE_FIRST,
            vec![
                (
                    "id".to_owned(),
                    Value::String(format!("{SERVER_KEY_PREFIX}1")),
                ),
                ("key".to_owned(), Value::Bytes(first.to_vec())),
            ],
        )
        .await;
    match created {
        Ok(_) => Ok(()),
        // Refused because another node created it first — but only if a key is there now.
        Err(refused @ MetaError::Refused { .. }) => {
            if ids(link).await?.is_empty() {
                Err(refused.into())
            } else {
                Ok(())
            }
        }
        Err(other) => Err(other.into()),
    }
}

async fn reveal(link: &mut VaultLink, id: String) -> Result<StoredKey> {
    let mut answers = link
        .run(
            "REVEAL key FROM s3_keys:$id;",
            vec![("id".to_owned(), Value::String(id.clone()))],
        )
        .await?;
    let Some(Value::Object(mut fields)) = first_record(answers.pop())? else {
        return Err(malformed("a reveal without fields"));
    };
    let Some(Value::Bytes(bytes)) = fields.remove("key") else {
        return Err(malformed("a key that is not bytes"));
    };
    let bytes = Zeroizing::new(bytes);
    let key: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| malformed("a key that is not 32 bytes"))?;
    Ok(StoredKey {
        id,
        key: Zeroizing::new(key),
    })
}

/// One open on one connection: unseal, make sure a key exists, reveal every key, seal.
async fn open_once(
    link: &mut VaultLink,
    passphrase: &Passphrase,
    first: &Zeroizing<[u8; 32]>,
) -> Result<Vec<StoredKey>> {
    if !declared_on(link).await? {
        declare(link, passphrase).await?;
    }
    link.run(FIELDS, Vec::new()).await?;
    unseal(link, passphrase).await?;
    let mut held = ids(link).await?;
    if held.is_empty() {
        create_first(link, first).await?;
        held = ids(link).await?;
    }
    let mut keys = Vec::with_capacity(held.len());
    for id in held {
        match reveal(link, id).await {
            Ok(key) => keys.push(key),
            // Another node sealed it after this one's unseal: the caller opens again.
            Err(Error::Meta(MetaError::Refused { .. }))
                if link.status().await?.state != SealState::Unsealed =>
            {
                return Err(Error::KeysSealed);
            }
            Err(error) => return Err(error),
        }
    }
    if let Err(error) = link.seal().await {
        // It seals itself at the end of its period; the keys are already in this node's memory either way.
        tracing::warn!(error = %error, "the keys vault was not sealed after reading; it lapses on its own");
    }
    Ok(keys)
}

#[path = "tessaridb_create.rs"]
mod create;
use create::create_once;

impl KeyVault for TessariKeys {
    async fn declared(&self) -> Result<bool> {
        let mut answers = self.pool.run("INFO FOR DATABASE;", Vec::new()).await?;
        Ok(names_vault(first_record(answers.pop())?))
    }

    async fn open(
        &self,
        passphrase: &Passphrase,
        first: &Zeroizing<[u8; 32]>,
    ) -> Result<Vec<StoredKey>> {
        let mut attempt: u32 = 1;
        loop {
            let mut link = self.pool.vault_link(VAULT).await?;
            match open_once(&mut link, passphrase, first).await {
                // Another node sealed it mid-read, or the declaration or the first key lost to another catalog
                // commit — which contends across the whole metadata store: open again after a wait, and when every
                // attempt lost, answer what the last one met.
                Err(
                    error @ (Error::KeysSealed
                    | Error::Meta(MetaError::Refused {
                        class: Some(RefusalClass::Retry),
                        ..
                    })),
                ) if attempt < KEYS_OPEN_ATTEMPTS => {
                    tracing::debug!(attempt, error = %error, "the keys vault was contended; opening again");
                    tokio::time::sleep(crate::backoff::backoff(attempt, KEYS_OPEN_BACKOFF_MS))
                        .await;
                    attempt = attempt.saturating_add(1);
                }
                answered => return answered,
            }
        }
    }

    async fn create(
        &self,
        passphrase: &Passphrase,
        id: &str,
        key: &Zeroizing<[u8; 32]>,
    ) -> Result<()> {
        let mut link = self.pool.vault_link(VAULT).await?;
        create_once(&mut link, passphrase, id, key).await
    }
}

fn malformed(reason: &'static str) -> Error {
    Error::Malformed {
        record: "keys vault",
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::CREATE_FIRST;

    #[test]
    fn the_first_key_is_created_never_overwritten() {
        assert!(CREATE_FIRST.starts_with("CREATE "), "{CREATE_FIRST}");
    }
}
