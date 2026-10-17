//! Temporary credentials: issued to a user for an identity the provider vouched for, presented with a session token
//! whose digest alone is stored, and dropped by the store at their expiry.

use tessari_s3_core::authz::UserName;

use super::super::credentials;
use super::super::model::{IssuedKey, NewSession};
use super::super::repository::UserRepository;
use super::super::sealer::{Binding, SessionBinding};
use super::{ID_ATTEMPTS, UserService};
use crate::{Error, Result};

impl UserService {
    /// New temporary credentials for `user`, its secret shown here once and stored only sealed with the session bound
    /// into it; `None` when there is no such user or it is disabled. Whether the identity may act as `user` is the
    /// caller's to decide before issuing.
    ///
    /// # Errors
    /// As [`UserService::issue_key`].
    pub async fn issue_session(
        &self,
        user: &UserName,
        session: &NewSession,
    ) -> Result<Option<IssuedKey>> {
        let sealer = self.sealer()?;
        let Some(holder) = self.get(user).await?.filter(|holder| !holder.disabled) else {
            return Ok(None);
        };
        let secret = credentials::secret()?;
        for _ in 0..ID_ATTEMPTS {
            let id = credentials::access_key_id()?;
            let binding = Binding {
                key_id: &id,
                user: holder.name.as_str(),
                space: holder.space.as_str(),
                scope: None,
                session: Some(SessionBinding {
                    expires: session.expires,
                    token_digest: &session.token_digest,
                    issuer: &session.issuer,
                    subject: &session.subject,
                }),
            };
            let sealed = sealer.seal(&secret, binding)?;
            if self
                .repository
                .insert_session_key(&id, &holder.name, &sealed, session)
                .await?
            {
                return Ok(Some(IssuedKey {
                    access_key_id: id,
                    secret,
                }));
            }
        }
        Err(Error::Randomness)
    }
}
