//! The steps every engine vault this server keeps is opened with: declared with its own passphrase, unsealed on the
//! node a connection reached, and listed by id. The keys vault and the secrets vault differ only in what they hold.

use tessari_s3_infrastructure::tessaridb::{
    MetaError, RecordId, RecordRef, RefusalClass, SealState, Value, VaultLink,
};
use zeroize::Zeroizing;

use crate::answers::first_record;
use crate::keys::Passphrase;
use crate::{Error, Result};

/// Most records one listing reads: one page.
const MAX_RECORDS: u32 = 10_000;

fn malformed(reason: &'static str) -> Error {
    Error::Malformed {
        record: "vault",
        reason,
    }
}

/// Whether the database `info` describes names `vault`.
pub(crate) fn names_vault(info: Option<Value>, vault: &str) -> bool {
    let Some(Value::Object(mut info)) = info else {
        return false;
    };
    match info.remove("vaults") {
        Some(Value::Array(names) | Value::Set(names)) => names
            .iter()
            .any(|name| matches!(name, Value::String(name) if name == vault)),
        Some(Value::Object(names)) => names.contains_key(vault),
        _ => false,
    }
}

/// Whether the database `link` selects has declared `vault`.
pub(crate) async fn declared_on(link: &mut VaultLink, vault: &str) -> Result<bool> {
    let mut answers = link.run("INFO FOR DATABASE;", Vec::new()).await?;
    Ok(names_vault(first_record(answers.pop())?, vault))
}

/// Declares the vault. The passphrase is the one literal this domain writes into statement text, because the engine
/// takes an own passphrase no other way (dogfood DB-22); [`Passphrase`] admits nothing that needs escaping, and a
/// refusal — whose text may quote the statement — is never passed on.
pub(crate) async fn declare(
    link: &mut VaultLink,
    vault: &str,
    passphrase: &Passphrase,
) -> Result<()> {
    let statement = Zeroizing::new(format!(
        "DEFINE VAULT {vault} PASSPHRASE '{}';",
        passphrase.expose()
    ));
    match link.run(&statement, Vec::new()).await {
        Ok(_) => Ok(()),
        // Another node declared it first; the caller reads it back.
        Err(MetaError::Refused { .. }) if declared_on(link, vault).await? => Ok(()),
        Err(MetaError::Refused { class, .. }) => Err(Error::Meta(MetaError::Refused {
            message: format!("the {vault} vault could not be declared"),
            class,
        })),
        Err(other) => Err(other.into()),
    }
}

/// Unseals the vault on the node `link` reached, unless it is already open there.
pub(crate) async fn unseal(link: &mut VaultLink, passphrase: &Passphrase) -> Result<()> {
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
pub(crate) async fn ids(link: &mut VaultLink, vault: &str) -> Result<Vec<String>> {
    let script = format!("INFO FOR VAULT {vault} RECORDS LIMIT {MAX_RECORDS};");
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
