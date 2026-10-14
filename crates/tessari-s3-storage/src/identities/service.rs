//! Identity rules: an in-flight login is taken once, and an identity — an issuer and a subject — signs in as at most
//! one user, each user holding at most one identity.

use sha2::{Digest, Sha256};
use tessari_s3_core::authz::UserName;
use tessari_s3_types::SecretKey;

use super::entity::LinkEntity;
use super::model::{Linked, PendingLogin};
use super::repository::{IdentityRepository, LinkWritten};
use super::tessaridb::TessariIdentities;
use crate::Result;

/// Login and identity-link operations.
#[derive(Clone)]
pub struct IdentityService {
    repository: TessariIdentities,
}

/// The record id of the identity (`issuer`, `subject`): a digest, so neither value is spelled into an id and the two
/// cannot run together.
fn link_id(issuer: &str, subject: &str) -> String {
    let digest = Sha256::new()
        .chain_update(issuer.as_bytes())
        .chain_update(b"\n")
        .chain_update(subject.as_bytes())
        .finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

impl IdentityService {
    /// The service over `repository`.
    #[must_use]
    pub const fn new(repository: TessariIdentities) -> Self {
        Self { repository }
    }

    /// Records `login` until its callback takes it or `lifetime_secs` pass.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn begin_login(&self, login: &PendingLogin, lifetime_secs: i64) -> Result<()> {
        self.repository.insert_login(login, lifetime_secs).await
    }

    /// The login `state` names, removed as it is read: `None` for a state never issued, expired or already taken.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn take_login(&self, state: &str) -> Result<Option<PendingLogin>> {
        Ok(self
            .repository
            .remove_login(state)
            .await?
            .map(|entity| PendingLogin {
                state: state.to_owned(),
                nonce: SecretKey::new(entity.nonce),
                verifier: SecretKey::new(entity.verifier),
            }))
    }

    /// The user the identity (`issuer`, `subject`) signs in as, if it is linked.
    ///
    /// # Errors
    /// The metadata store's refusal or outage, or a stored name that is not a user name.
    pub async fn user_for(&self, issuer: &str, subject: &str) -> Result<Option<UserName>> {
        let id = link_id(issuer, subject);
        self.repository
            .get_link(&id)
            .await?
            .map(|link| {
                UserName::new(&link.user_name).ok_or(crate::Error::Malformed {
                    record: "identity link",
                    reason: "user_name",
                })
            })
            .transpose()
    }

    /// Links `user` to the identity (`issuer`, `subject`), replacing the user's earlier link; an identity another
    /// user holds is left with them.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn link(&self, user: &UserName, issuer: &str, subject: &str) -> Result<Linked> {
        let id = link_id(issuer, subject);
        let held_by_another =
            |link: Option<LinkEntity>| link.is_some_and(|link| link.user_name != user.as_str());
        if held_by_another(self.repository.get_link(&id).await?) {
            return Ok(Linked::TakenByAnother);
        }
        let link = LinkEntity {
            issuer: issuer.to_owned(),
            subject: subject.to_owned(),
            user_name: user.as_str().to_owned(),
        };
        match self.repository.replace_link(&id, user, &link).await? {
            LinkWritten::Written => Ok(Linked::Linked),
            // A concurrent write won; whoever holds the identity now decides the answer, and nobody holding it means
            // the race is not over.
            LinkWritten::Lost => match self.repository.get_link(&id).await? {
                Some(link) if link.user_name == user.as_str() => Ok(Linked::Linked),
                Some(_) => Ok(Linked::TakenByAnother),
                None => Err(crate::Error::Contended),
            },
        }
    }

    /// Removes `user`'s link; `false` when it had none.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn unlink(&self, user: &UserName) -> Result<bool> {
        let mut removed = false;
        for link in self.repository.links_of(user).await? {
            removed |= self
                .repository
                .remove_link(&link_id(&link.issuer, &link.subject))
                .await?;
        }
        Ok(removed)
    }

    /// Ends `user`'s console sessions: every one issued at or before `at` is refused from its next request.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn end_sessions(&self, user: &UserName, at: i64) -> Result<()> {
        self.repository.put_logout(user, at).await
    }

    /// When `user`'s console sessions were last ended, if ever.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn sessions_ended(&self, user: &UserName) -> Result<Option<i64>> {
        self.repository.get_logout(user).await
    }

    /// Takes the logout token `jti` for acting on, kept `keep_secs` so it is acted on once: `false` when it was
    /// already taken, also by a concurrent delivery.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn take_logout(&self, jti: &str, keep_secs: i64) -> Result<bool> {
        self.repository.insert_logout_id(jti, keep_secs).await
    }
}
