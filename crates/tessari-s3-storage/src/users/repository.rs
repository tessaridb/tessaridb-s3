//! What user, access-key and grant persistence must answer, whatever stores it.

use std::future::Future;

use tessari_s3_core::authz::{Access, UserName, Visible};
use tessari_s3_types::BucketName;

use super::entity::{AccessKeyEntity, GrantEntity, UserEntity};
use super::model::{KeyScope, NewUser};
use super::sealer::Sealed;
use crate::Result;

/// The outcome of inserting a user record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Inserted {
    /// The record was written.
    Created(UserEntity),
    /// A user of that name already exists.
    Exists,
}

/// User, access-key and grant queries; no rule beyond what a query states.
pub(crate) trait UserRepository: Send + Sync {
    /// Writes a new, enabled user record.
    fn insert(&self, user: &NewUser) -> impl Future<Output = Result<Inserted>> + Send;
    /// The record of `name`, if any.
    fn get(&self, name: &UserName) -> impl Future<Output = Result<Option<UserEntity>>> + Send;
    /// Every user record `visible` covers, filtered in the query.
    fn list(&self, visible: &Visible) -> impl Future<Output = Result<Vec<UserEntity>>> + Send;
    /// Sets an existing user's `disabled` flag.
    fn set_disabled(
        &self,
        name: &UserName,
        disabled: bool,
    ) -> impl Future<Output = Result<()>> + Send;
    /// Writes a new, enabled access key `id` for `user` — a one-key credential when `scope` is given, which the store
    /// then drops at its expiry; `false` when that id is taken.
    fn insert_key(
        &self,
        id: &str,
        user: &UserName,
        sealed: &Sealed,
        scope: Option<&KeyScope>,
    ) -> impl Future<Output = Result<bool>> + Send;
    /// The record of access key `id`, if any.
    fn get_key(&self, id: &str) -> impl Future<Output = Result<Option<AccessKeyEntity>>> + Send;
    /// Sets an existing key's `disabled` flag.
    fn set_key_disabled(&self, id: &str, disabled: bool)
    -> impl Future<Output = Result<()>> + Send;
    /// Removes key `id`; `false` when there was none.
    fn remove_key(&self, id: &str) -> impl Future<Output = Result<bool>> + Send;
    /// Writes `user`'s grant on `bucket`, replacing any it had.
    fn put_grant(
        &self,
        user: &UserName,
        bucket: &BucketName,
        access: Access,
    ) -> impl Future<Output = Result<()>> + Send;
    /// Removes `user`'s grant on `bucket`; `false` when there was none.
    fn remove_grant(
        &self,
        user: &UserName,
        bucket: &BucketName,
    ) -> impl Future<Output = Result<bool>> + Send;
    /// Every grant `user` holds.
    fn grants(&self, user: &UserName) -> impl Future<Output = Result<Vec<GrantEntity>>> + Send;
}
