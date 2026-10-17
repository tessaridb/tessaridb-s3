//! Group rules: a group belongs to one space and so do its members and the buckets it is granted; removing a group
//! takes its memberships and grants with it; a member receives the group's grants as if they were their own.

use tessari_s3_core::authz::{Access, GroupName, SpaceName, UserName};
use tessari_s3_types::BucketName;

use super::model::{Group, GroupGrant, MemberChange};
use super::repository::{GroupRepository, Touched};
use super::tessaridb::TessariGroups;
use crate::Result;

/// The most groups one listing of a space answers.
const LIST_LIMIT: usize = 1000;

/// Group, membership and group-grant operations.
#[derive(Clone)]
pub struct GroupService {
    repository: TessariGroups,
}

/// A group's record id: its space and its name, neither of which may hold a `/`.
fn group_id(space: &SpaceName, name: &GroupName) -> String {
    format!("{}/{}", space.as_str(), name.as_str())
}

impl GroupService {
    /// The service over `repository`.
    #[must_use]
    pub const fn new(repository: TessariGroups) -> Self {
        Self { repository }
    }

    /// Creates group `name` in `space`; `false` when it exists. Whether the space exists is the caller's to know.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn create(&self, space: &SpaceName, name: &GroupName) -> Result<bool> {
        self.repository
            .insert_group(&group_id(space, name), space, name.as_str())
            .await
    }

    /// Group `name` of `space`, if there is one.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn get(&self, space: &SpaceName, name: &GroupName) -> Result<Option<Group>> {
        self.repository
            .get_group(&group_id(space, name))
            .await?
            .map(Group::try_from)
            .transpose()
    }

    /// The groups of `space`, in byte order of their names.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn list(&self, space: &SpaceName) -> Result<Vec<Group>> {
        let mut groups = self
            .repository
            .groups_in(space, LIST_LIMIT)
            .await?
            .into_iter()
            .map(Group::try_from)
            .collect::<Result<Vec<_>>>()?;
        groups.sort_by(|a, b| a.name.as_str().as_bytes().cmp(b.name.as_str().as_bytes()));
        Ok(groups)
    }

    /// Removes group `name` of `space` with its memberships and grants; `false` when there is no such group.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn delete(&self, space: &SpaceName, name: &GroupName) -> Result<bool> {
        let id = group_id(space, name);
        if self.repository.get_group(&id).await?.is_none() {
            return Ok(false);
        }
        self.repository.remove_group(&id).await?;
        Ok(true)
    }

    /// Adds `user` of `user_space` to group `name` of `space`; a user of another space is refused.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn add_member(
        &self,
        space: &SpaceName,
        name: &GroupName,
        user: &UserName,
        user_space: &SpaceName,
    ) -> Result<MemberChange> {
        if user_space != space {
            return Ok(MemberChange::OtherSpace);
        }
        let id = group_id(space, name);
        let member = format!("{id}/{}", user.as_str());
        let touched = self.repository.put_member(&id, &member, user).await?;
        self.settled(&id, touched).await
    }

    /// Removes `user` from group `name` of `space`; `false` when they were not a member.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn remove_member(
        &self,
        space: &SpaceName,
        name: &GroupName,
        user: &UserName,
    ) -> Result<bool> {
        let member = format!("{}/{}", group_id(space, name), user.as_str());
        self.repository.remove_member(&member).await
    }

    /// The members of group `name` of `space`, in byte order.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn members(&self, space: &SpaceName, name: &GroupName) -> Result<Vec<String>> {
        let mut members: Vec<String> = self
            .repository
            .members(&group_id(space, name))
            .await?
            .into_iter()
            .map(|member| member.user_name)
            .collect();
        members.sort_unstable_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        Ok(members)
    }

    /// Grants `bucket` of `bucket_space` to group `name` of `space`; a bucket of another space is refused.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn set_grant(
        &self,
        space: &SpaceName,
        name: &GroupName,
        bucket: &BucketName,
        bucket_space: &SpaceName,
        access: Access,
    ) -> Result<MemberChange> {
        if bucket_space != space {
            return Ok(MemberChange::OtherSpace);
        }
        let id = group_id(space, name);
        let grant = format!("{id}/{}", bucket.as_str());
        let touched = self
            .repository
            .put_grant(&id, &grant, bucket.as_str(), access)
            .await?;
        self.settled(&id, touched).await
    }

    /// Removes group `name`'s grant on `bucket`; `false` when it had none.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn remove_grant(
        &self,
        space: &SpaceName,
        name: &GroupName,
        bucket: &BucketName,
    ) -> Result<bool> {
        let grant = format!("{}/{}", group_id(space, name), bucket.as_str());
        self.repository.remove_grant(&grant).await
    }

    /// The grants of group `name` of `space`, in byte order of their buckets.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn grants(&self, space: &SpaceName, name: &GroupName) -> Result<Vec<GroupGrant>> {
        let mut grants: Vec<GroupGrant> = self
            .repository
            .grants(&group_id(space, name))
            .await?
            .into_iter()
            .map(GroupGrant::from)
            .collect();
        grants.sort_by(|a, b| a.bucket.as_bytes().cmp(b.bucket.as_bytes()));
        Ok(grants)
    }

    /// What `user` of `space` receives from their groups: each bucket once, the union of the groups granting it. Only
    /// groups of the user's own space count, whatever a membership record says.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn received(&self, user: &UserName, space: &SpaceName) -> Result<Vec<GroupGrant>> {
        let own_space = format!("{}/", space.as_str());
        let mut received: Vec<GroupGrant> = Vec::new();
        for member in self.repository.memberships(user).await? {
            if !member.group_id.starts_with(&own_space) {
                continue;
            }
            for grant in self.repository.grants(&member.group_id).await? {
                let grant = GroupGrant::from(grant);
                match received.iter_mut().find(|held| held.bucket == grant.bucket) {
                    Some(held) => held.access = held.access.union(grant.access),
                    None => received.push(grant),
                }
            }
        }
        Ok(received)
    }

    /// A touching write's outcome: a refusal means the group was not there when it was written — re-read to be sure,
    /// so a refusal for any other reason is not mistaken for a missing group.
    async fn settled(&self, id: &str, touched: Touched) -> Result<MemberChange> {
        match touched {
            Touched::Written => Ok(MemberChange::Done),
            Touched::Refused(error) => {
                if self.repository.get_group(id).await?.is_none() {
                    Ok(MemberChange::NoSuchGroup)
                } else {
                    Err(error.into())
                }
            }
        }
    }
}
