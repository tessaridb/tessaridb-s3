//! A user's full keys as the console manages them: issued at most once per form token, listed without secrets, and
//! revoked only by the user they belong to.

use tessari_s3_constants::{
    KEY_ISSUE_SETTLE_FIRST_MS, KEY_ISSUE_SETTLE_READS, KEY_ISSUE_TOKEN_SECS, USER_KEYS_LISTED_MAX,
};
use tessari_s3_core::authz::UserName;
use tessari_s3_types::Timestamp;

use super::super::credentials;
use super::super::entity::IssueEntity;
use super::super::model::{IssueToken, IssuedKey, IssuedOnce, ListedKey, scope_from};
use super::super::repository::UserRepository;
use super::super::sealer::Binding;
use super::{ID_ATTEMPTS, UserService};
use crate::{Error, Result};

impl UserService {
    /// A new full key for `user` under `token`, at most one per token: a token that already issued `user` a key
    /// answers that key's id and no secret, and one that issued another user a key issues nothing. The token is
    /// remembered for [`KEY_ISSUE_TOKEN_SECS`]; two submissions racing on it settle on its record, so one key results.
    ///
    /// # Errors
    /// As [`UserService::issue_key`].
    pub async fn issue_key_once(&self, user: &UserName, token: &IssueToken) -> Result<IssuedOnce> {
        let sealer = self.sealer()?;
        if let Some(prior) = self.repository.get_issue(token).await? {
            return Ok(Self::prior(&prior, user));
        }
        let Some(holder) = self.get(user).await? else {
            return Ok(IssuedOnce::NoSuchUser);
        };
        let secret = credentials::secret()?;
        for _ in 0..ID_ATTEMPTS {
            let id = credentials::access_key_id()?;
            let binding = Binding {
                key_id: &id,
                user: holder.name.as_str(),
                space: holder.space.as_str(),
                scope: None,
                session: None,
            };
            let sealed = sealer.seal(&secret, binding)?;
            let written = self
                .repository
                .insert_key_once(&id, &holder.name, &sealed, token, KEY_ISSUE_TOKEN_SECS)
                .await?;
            if written {
                return Ok(IssuedOnce::Issued(IssuedKey {
                    access_key_id: id,
                    secret,
                }));
            }
            // Taken: either the token (a racing submission won, and its key is the answer) or the id (draw again).
            if let Some(prior) = self.settled_issue(token).await? {
                return Ok(Self::prior(&prior, user));
            }
        }
        Err(Error::Randomness)
    }

    /// The issue `token` names, read until the submission that won it is readable: its commit is refused to the
    /// loser before it can be read, so one read straight after the refusal can miss it. `None` when it never lands —
    /// the winner did not commit, or the id was what collided — and a new id is drawn.
    async fn settled_issue(&self, token: &IssueToken) -> Result<Option<IssueEntity>> {
        let mut wait = KEY_ISSUE_SETTLE_FIRST_MS;
        for read in 1..=KEY_ISSUE_SETTLE_READS {
            if let Some(prior) = self.repository.get_issue(token).await? {
                return Ok(Some(prior));
            }
            if read < KEY_ISSUE_SETTLE_READS {
                tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
                wait = wait.saturating_mul(2);
            }
        }
        Ok(None)
    }

    fn prior(prior: &IssueEntity, user: &UserName) -> IssuedOnce {
        if prior.user_name == user.as_str() {
            IssuedOnce::AlreadyIssued(prior.key_id.clone())
        } else {
            IssuedOnce::TokenReused
        }
    }

    /// `user`'s keys, at most [`USER_KEYS_LISTED_MAX`], without their secrets.
    ///
    /// # Errors
    /// [`Error::Malformed`] for a record that does not read; the metadata store's refusal or outage.
    pub async fn keys_of(&self, user: &UserName) -> Result<Vec<ListedKey>> {
        self.repository
            .keys_of(user, USER_KEYS_LISTED_MAX)
            .await?
            .into_iter()
            .map(|key| {
                let session = key.session_issuer.is_some();
                let scope = scope_from(
                    key.scope_bucket.as_ref(),
                    key.scope_key.as_ref(),
                    key.expires,
                    session,
                )?;
                let (seconds, nanos) = key.created;
                Ok(ListedKey {
                    access_key_id: key.id,
                    created: Timestamp { seconds, nanos },
                    disabled: key.disabled,
                    scope,
                    session_expires: key.expires.filter(|_| session).map(|(at, _)| at),
                })
            })
            .collect()
    }

    /// Removes `user`'s key `id`; `false` when there is no such key or it is another user's, alike, so the answer
    /// does not say which. A key's holder never changes, so reading it and then removing it cannot cross users.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn revoke_key_of(&self, user: &UserName, id: &str) -> Result<bool> {
        if !credentials::is_access_key_id(id) {
            return Ok(false);
        }
        match self.repository.get_key(id).await? {
            Some(key) if key.user_name == user.as_str() => self.repository.remove_key(id).await,
            _ => Ok(false),
        }
    }
}
