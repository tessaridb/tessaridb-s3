//! What group persistence must answer, whatever stores it.

use std::future::Future;

use tessari_s3_core::authz::{Access, SpaceName, UserName};

use super::entity::{GroupEntity, GroupGrantEntity, MemberEntity};
use crate::Result;

/// The outcome of a write that names its group.
#[derive(Debug)]
pub(crate) enum Touched {
    /// Written, the group touched in the same transaction.
    Written,
    /// The store refused it — what the touch of a missing group answers, and what a lost race answers; kept to be
    /// returned when the group turns out to be there.
    Refused(tessari_s3_infrastructure::tessaridb::MetaError),
}

/// Group, membership and group-grant queries; no rule beyond what a query states.
pub(crate) trait GroupRepository: Send + Sync {
    /// Creates group `id`; `false` when it exists or a concurrent create won.
    fn insert_group(
        &self,
        id: &str,
        space: &SpaceName,
        name: &str,
    ) -> impl Future<Output = Result<bool>> + Send;
    /// The group stored as `id`, if any.
    fn get_group(&self, id: &str) -> impl Future<Output = Result<Option<GroupEntity>>> + Send;
    /// Every group of `space`, at most `limit`.
    fn groups_in(
        &self,
        space: &SpaceName,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<GroupEntity>>> + Send;
    /// Removes group `id` with its memberships and grants, all or nothing.
    fn remove_group(&self, id: &str) -> impl Future<Output = Result<()>> + Send;
    /// Writes membership `member_id` of `user` in group `group_id`, touching the group in the same transaction.
    fn put_member(
        &self,
        group_id: &str,
        member_id: &str,
        user: &UserName,
    ) -> impl Future<Output = Result<Touched>> + Send;
    /// Removes membership `member_id`; `true` when it was there.
    fn remove_member(&self, member_id: &str) -> impl Future<Output = Result<bool>> + Send;
    /// The memberships of group `group_id`.
    fn members(&self, group_id: &str) -> impl Future<Output = Result<Vec<MemberEntity>>> + Send;
    /// The memberships of `user`.
    fn memberships(
        &self,
        user: &UserName,
    ) -> impl Future<Output = Result<Vec<MemberEntity>>> + Send;
    /// Writes grant `grant_id` of `bucket` to group `group_id`, touching the group in the same transaction.
    fn put_grant(
        &self,
        group_id: &str,
        grant_id: &str,
        bucket: &str,
        access: Access,
    ) -> impl Future<Output = Result<Touched>> + Send;
    /// Removes grant `grant_id`; `true` when it was there.
    fn remove_grant(&self, grant_id: &str) -> impl Future<Output = Result<bool>> + Send;
    /// The grants of group `group_id`.
    fn grants(&self, group_id: &str) -> impl Future<Output = Result<Vec<GroupGrantEntity>>> + Send;
}
