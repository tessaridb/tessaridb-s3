//! The settings in TessariDB, in the global database: plain settings in `s3_settings`, one record per variable, and
//! secrets in the vault `s3_secrets`, declared with the keys passphrase as its own so no authority in the store opens
//! it. Every read of a secret is a `REVEAL`, which the engine records before it answers.

use std::collections::HashMap;

use tessari_s3_constants::{KEYS_OPEN_ATTEMPTS, KEYS_OPEN_BACKOFF_MS};
use tessari_s3_infrastructure::tessaridb::{
    MetaError, MetaPool, RefusalClass, SealState, Value, VaultLink,
};
use zeroize::Zeroizing;

use super::repository::SettingsStore;
use crate::answers::{all_records, first_record};
use crate::keys::Passphrase;
use crate::vaults;
use crate::{Error, Result};

/// The secrets vault's name.
const VAULT: &str = "s3_secrets";
/// The plain settings' table; re-run before every read, so the first node to start declares it.
const TABLE: &str =
    "DEFINE TABLE IF NOT EXISTS s3_settings (name string REQUIRED, value string REQUIRED);";
/// The secrets' field; re-run on every open, so a node that stopped between declaring the vault and its field
/// converges.
const FIELDS: &str = "DEFINE FIELD IF NOT EXISTS value ON s3_secrets TYPE string SECRET;";
/// Far more settings than this build keeps, so one read takes them all.
const MAX_SETTINGS: u32 = 1_000;

/// The TessariDB settings store.
#[derive(Clone)]
pub struct TessariSettings {
    /// The global database.
    pool: MetaPool,
}

impl TessariSettings {
    /// The settings in the database `pool` selects (the global one).
    #[must_use]
    pub const fn new(pool: MetaPool) -> Self {
        Self { pool }
    }
}

fn string(name: &str, value: &str) -> (String, Value) {
    (name.to_owned(), Value::String(value.to_owned()))
}

/// `Ok(())` for a create that landed or lost to one already there: the first value stored is the one kept.
fn kept<T>(created: tessari_s3_infrastructure::tessaridb::MetaResult<T>) -> Result<()> {
    match created {
        Ok(_) => Ok(()),
        Err(error) if error.is_class(RefusalClass::Conflict) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn malformed(reason: &'static str) -> Error {
    Error::Malformed {
        record: "setting",
        reason,
    }
}

/// Declares the vault and its field if absent, and unseals it on the node `link` reached.
async fn opened(link: &mut VaultLink, passphrase: &Passphrase) -> Result<()> {
    if !vaults::declared_on(link, VAULT).await? {
        vaults::declare(link, VAULT, passphrase).await?;
    }
    link.run(FIELDS, Vec::new()).await?;
    vaults::unseal(link, passphrase).await
}

async fn seal(link: &mut VaultLink) {
    if let Err(error) = link.seal().await {
        // It seals itself at the end of its period; what was read is already in this node's memory either way.
        tracing::warn!(error = %error, "the secrets vault was not sealed after reading; it lapses on its own");
    }
}

/// One open on one connection: every secret the vault holds.
async fn read_once(
    link: &mut VaultLink,
    passphrase: &Passphrase,
) -> Result<HashMap<String, Zeroizing<String>>> {
    opened(link, passphrase).await?;
    let mut secrets = HashMap::new();
    for id in vaults::ids(link, VAULT).await? {
        let revealed = link
            .run("REVEAL value FROM s3_secrets:$id;", vec![string("id", &id)])
            .await;
        let mut answers = match revealed {
            Ok(answers) => answers,
            // Another node sealed it after this one's unseal: the caller opens again.
            Err(MetaError::Refused { .. }) if link.status().await?.state != SealState::Unsealed => {
                return Err(Error::KeysSealed);
            }
            Err(error) => return Err(error.into()),
        };
        let Some(Value::Object(mut fields)) = first_record(answers.pop())? else {
            return Err(malformed("a reveal without fields"));
        };
        let Some(Value::String(value)) = fields.remove("value") else {
            return Err(malformed("a secret that is not text"));
        };
        secrets.insert(id, Zeroizing::new(value));
    }
    seal(link).await;
    Ok(secrets)
}

impl SettingsStore for TessariSettings {
    async fn settings(&self) -> Result<HashMap<String, String>> {
        self.pool.run(TABLE, Vec::new()).await?;
        let mut answers = self
            .pool
            .run(
                &format!("SELECT * FROM s3_settings LIMIT {MAX_SETTINGS};"),
                Vec::new(),
            )
            .await?;
        all_records(answers.pop(), "setting")?
            .into_iter()
            .map(|record| match record {
                Value::Object(mut fields) => {
                    match (fields.remove("name"), fields.remove("value")) {
                        (Some(Value::String(name)), Some(Value::String(value))) => {
                            Ok((name, value))
                        }
                        _ => Err(malformed("a setting without a text name and value")),
                    }
                }
                _ => Err(malformed("a setting that is not an object")),
            })
            .collect()
    }

    async fn seed_setting(&self, id: &str, value: &str) -> Result<()> {
        self.pool.run(TABLE, Vec::new()).await?;
        kept(
            self.pool
                .run(
                    "CREATE s3_settings:$id = { name: $id, value: $value };",
                    vec![string("id", id), string("value", value)],
                )
                .await,
        )
    }

    async fn secrets(&self, passphrase: &Passphrase) -> Result<HashMap<String, Zeroizing<String>>> {
        let mut attempt: u32 = 1;
        loop {
            let mut link = self.pool.vault_link(VAULT).await?;
            match read_once(&mut link, passphrase).await {
                // Another node sealed it mid-read, or the declaration lost to another catalog commit: open again
                // after a wait, and when every attempt lost, answer what the last one met.
                Err(
                    error @ (Error::KeysSealed
                    | Error::Meta(MetaError::Refused {
                        class: Some(RefusalClass::Retry),
                        ..
                    })),
                ) if attempt < KEYS_OPEN_ATTEMPTS => {
                    tracing::debug!(attempt, error = %error, "the secrets vault was contended; opening again");
                    tokio::time::sleep(crate::backoff::backoff(attempt, KEYS_OPEN_BACKOFF_MS))
                        .await;
                    attempt = attempt.saturating_add(1);
                }
                answered => return answered,
            }
        }
    }

    async fn seed_secret(&self, passphrase: &Passphrase, id: &str, value: &str) -> Result<()> {
        let mut link = self.pool.vault_link(VAULT).await?;
        opened(&mut link, passphrase).await?;
        let created = link
            .run(
                "CREATE s3_secrets:$id = { value: $value };",
                vec![string("id", id), string("value", value)],
            )
            .await;
        seal(&mut link).await;
        kept(created)
    }
}
