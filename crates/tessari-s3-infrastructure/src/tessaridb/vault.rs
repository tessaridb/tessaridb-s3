//! One connection for a vault. An unseal is held by ONE engine process, so the unseal, the reads that need it and the
//! seal go over a single connection: pooled ones may reach another node of the cluster between them. The connection
//! is never handed back to the pool — it is closed when dropped — and a passphrase travels in the vault frame as a
//! field, never in statement text.

use tessaridb_client::{Answer, Client, Value, Vault, VaultStatus};
use tokio::io::{AsyncRead, AsyncWrite};

use super::{Link, MetaPool, closed};
use crate::tessaridb::error::{MetaError, MetaResult};
use crate::tessaridb::settings::is_safe_name;

/// What a vault call does.
#[derive(Clone, Copy)]
enum Act<'a> {
    Status,
    Unseal(&'a str),
    Seal,
}

/// A signed-in connection for the vault `vault` in the pool's namespace and the database of the handle that opened
/// it.
pub struct VaultLink {
    link: Link,
    scope: String,
    namespace: String,
    database: String,
    vault: String,
}

impl MetaPool {
    /// A new connection, signed in, for vault `vault`.
    ///
    /// # Errors
    /// [`MetaError::Configuration`] when `vault` is not a name; the store's refusal of the sign-in, or
    /// [`MetaError::Unavailable`].
    pub async fn vault_link(&self, vault: &str) -> MetaResult<VaultLink> {
        if !is_safe_name(vault) {
            return Err(MetaError::Configuration {
                reason: "a vault name must be a plain lowercase name",
            });
        }
        let _signing = self
            .inner
            .signing_in
            .acquire()
            .await
            .map_err(|_| closed())?;
        let mut link = self.dial().await?;
        let (user, password) = self.credentials();
        let deadline = self.inner.settings.statement_timeout;
        let signed = tokio::time::timeout(
            deadline,
            link.run(&self.scope, Some((&user, &password)), Vec::new()),
        )
        .await
        .map_err(|_| unanswered(deadline))?;
        signed.map_err(meta_error)?;
        let (namespace, database) = self.scope();
        Ok(VaultLink {
            link,
            scope: self.scope.to_string(),
            namespace: namespace.to_owned(),
            database: database.to_owned(),
            vault: vault.to_owned(),
        })
    }
}

impl VaultLink {
    /// The vault's seal state on the node this connection reached.
    ///
    /// # Errors
    /// The node's refusal, with its class; the transport failure.
    pub async fn status(&mut self) -> MetaResult<VaultStatus> {
        self.act(Act::Status).await
    }

    /// Unseals the vault with `passphrase` on that node, for the node's period.
    ///
    /// # Errors
    /// As [`Self::status`]; a wrong passphrase is the node's refusal of class `Invalid`.
    pub async fn unseal(&mut self, passphrase: &str) -> MetaResult<VaultStatus> {
        self.act(Act::Unseal(passphrase)).await
    }

    /// Seals the vault on that node.
    ///
    /// # Errors
    /// As [`Self::status`].
    pub async fn seal(&mut self) -> MetaResult<VaultStatus> {
        self.act(Act::Seal).await
    }

    /// Runs `script` in the configured namespace and database over this connection, with `parameters` bound; the
    /// scoping `USE` is not among the answers.
    ///
    /// # Errors
    /// As [`MetaPool::run`].
    pub async fn run(
        &mut self,
        script: &str,
        parameters: Vec<(String, Value)>,
    ) -> MetaResult<Vec<Answer>> {
        // The one statement carrying a passphrase (a vault's declaration) passes through here; the copy is zeroed.
        let scoped = zeroize::Zeroizing::new(format!("{}{script}", self.scope));
        let mut answers = self
            .link
            .run(&scoped, None, parameters)
            .await
            .map_err(meta_error)?;
        if answers.is_empty() {
            return Err(MetaError::Protocol {
                reason: "the scoping USE was not answered",
            });
        }
        answers.remove(0);
        Ok(answers)
    }

    async fn act(&mut self, act: Act<'_>) -> MetaResult<VaultStatus> {
        let within = (self.namespace.as_str(), self.database.as_str());
        match &mut self.link {
            Link::Plain(client) => vault_act(client, within, &self.vault, act).await,
            Link::Secured(client) => vault_act(client.as_mut(), within, &self.vault, act).await,
        }
    }
}

async fn vault_act<S>(
    client: &mut Client<S>,
    within: (&str, &str),
    vault: &str,
    act: Act<'_>,
) -> MetaResult<VaultStatus>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut handle = Vault::new(client, within, vault).map_err(|_| MetaError::Configuration {
        reason: "a vault's namespace, database or name is not a name",
    })?;
    let answered = match act {
        Act::Status => handle.status().await,
        Act::Unseal(passphrase) => handle.unseal(passphrase).await,
        Act::Seal => handle.seal().await,
    };
    answered.map_err(meta_error)
}

/// A client failure as the pool reports it: a refusal keeps the node's words and class.
fn meta_error(error: tessaridb_client::Error) -> MetaError {
    match error {
        tessaridb_client::Error::Refused { message, class } => {
            MetaError::Refused { message, class }
        }
        other => MetaError::Unavailable {
            reason: other.to_string(),
        },
    }
}

fn unanswered(deadline: std::time::Duration) -> MetaError {
    MetaError::Unavailable {
        reason: format!(
            "the metadata store did not answer within {} s",
            deadline.as_secs()
        ),
    }
}
