//! What login and identity-link persistence must answer, whatever stores it.

use std::future::Future;

use tessari_s3_core::authz::UserName;

use super::entity::{LinkEntity, LoginEntity};
use super::model::PendingLogin;
use crate::Result;

/// The outcome of writing a user's link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LinkWritten {
    /// The link was written, replacing the user's earlier one.
    Written,
    /// The link record exists and the write lost: another user holds it, or a concurrent write won.
    Lost,
}

/// Login and identity-link queries; no rule beyond what a query states.
pub(crate) trait IdentityRepository: Send + Sync {
    /// Writes `login`, dropped by the store `lifetime_secs` from now.
    fn insert_login(
        &self,
        login: &PendingLogin,
        lifetime_secs: i64,
    ) -> impl Future<Output = Result<()>> + Send;
    /// Removes the login `state` names and answers what it held, in one statement; `None` when there is none, or a
    /// concurrent removal won.
    fn remove_login(&self, state: &str)
    -> impl Future<Output = Result<Option<LoginEntity>>> + Send;
    /// The link stored as `id`, if any.
    fn get_link(&self, id: &str) -> impl Future<Output = Result<Option<LinkEntity>>> + Send;
    /// Removes every link of `user` and writes `link` as `id`, both or neither.
    fn replace_link(
        &self,
        id: &str,
        user: &UserName,
        link: &LinkEntity,
    ) -> impl Future<Output = Result<LinkWritten>> + Send;
    /// Every link of `user`.
    fn links_of(&self, user: &UserName) -> impl Future<Output = Result<Vec<LinkEntity>>> + Send;
    /// Removes the link stored as `id`; `true` when it was there.
    fn remove_link(&self, id: &str) -> impl Future<Output = Result<bool>> + Send;
    /// Records that `user`'s console sessions ended at `at`.
    fn put_logout(&self, user: &UserName, at: i64) -> impl Future<Output = Result<()>> + Send;
    /// When `user`'s console sessions were last ended, if ever.
    fn get_logout(&self, user: &UserName) -> impl Future<Output = Result<Option<i64>>> + Send;
    /// Records the logout token `jti`, kept `keep_secs`; `false` when it is already recorded or a concurrent write won.
    fn insert_logout_id(
        &self,
        jti: &str,
        keep_secs: i64,
    ) -> impl Future<Output = Result<bool>> + Send;
}
