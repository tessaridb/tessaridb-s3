//! Groups in TessariDB: `groups:<space>/<name>`, `group_members:<space>/<name>/<user>` and
//! `group_grants:<space>/<name>/<bucket>`, the ids built by the service from validated names; every value is bound.
//! A membership or grant is written in the transaction that touches its group, so it conflicts with the group's
//! removal rather than outliving it.

use tessari_s3_core::authz::{Access, SpaceName, UserName};
use tessari_s3_infrastructure::tessaridb::{MetaPool, RefusalClass, Value};

use super::entity::{GroupEntity, GroupGrantEntity, MemberEntity};
use super::repository::{GroupRepository, Touched};
use crate::Result;
use crate::answers::{all_records, first_record};

/// The TessariDB groups repository.
#[derive(Clone)]
pub struct TessariGroups {
    pool: MetaPool,
}

impl TessariGroups {
    /// A repository over `pool`.
    #[must_use]
    pub const fn new(pool: MetaPool) -> Self {
        Self { pool }
    }

    async fn records(
        &self,
        script: &str,
        parameters: Vec<(String, Value)>,
        record: &'static str,
    ) -> Result<Vec<Value>> {
        let answers = self.pool.run(script, parameters).await?;
        all_records(answers.into_iter().next(), record)
    }

    async fn touching(&self, script: &str, parameters: Vec<(String, Value)>) -> Result<Touched> {
        match self.pool.run(script, parameters).await {
            Ok(_) => Ok(Touched::Written),
            // The touch of a group that is not there is refused (`conflict` on 0.34.2-beta: "no record …"), as is the
            // loser of a race over the group; the caller re-reads to tell the two apart.
            Err(error)
                if error.is_class(RefusalClass::Conflict)
                    || error.is_class(RefusalClass::Retry)
                    || error.is_class(RefusalClass::Invalid) =>
            {
                Ok(Touched::Refused(error))
            }
            Err(error) => Err(error.into()),
        }
    }
}

fn text(name: &str, value: &str) -> (String, Value) {
    (name.to_owned(), Value::String(value.to_owned()))
}

fn flag(name: &str, value: bool) -> (String, Value) {
    (name.to_owned(), Value::Bool(value))
}

impl GroupRepository for TessariGroups {
    async fn insert_group(&self, id: &str, space: &SpaceName, name: &str) -> Result<bool> {
        let parameters = vec![
            text("id", id),
            text("space", space.as_str()),
            text("name", name),
        ];
        match self
            .pool
            .run(
                "CREATE groups:$id = { space_name: $space, name: $name, created: time::now() };",
                parameters,
            )
            .await
        {
            Ok(_) => Ok(true),
            Err(error)
                if error.is_class(RefusalClass::Conflict)
                    || error.is_class(RefusalClass::Retry) =>
            {
                Ok(false)
            }
            Err(error) => Err(error.into()),
        }
    }

    async fn get_group(&self, id: &str) -> Result<Option<GroupEntity>> {
        let answers = self
            .pool
            .run("SELECT * FROM ONLY groups:$id;", vec![text("id", id)])
            .await?;
        first_record(answers.into_iter().next())?
            .map(|value| GroupEntity::from_value(&value))
            .transpose()
    }

    async fn groups_in(&self, space: &SpaceName, count: usize) -> Result<Vec<GroupEntity>> {
        self.records(
            "SELECT * FROM groups WHERE space_name = $space LIMIT $limit USING INDEX by_group_space;",
            vec![
                text("space", space.as_str()),
                ("limit".to_owned(), crate::params::count(count)),
            ],
            "group",
        )
        .await?
        .iter()
        .map(GroupEntity::from_value)
        .collect()
    }

    async fn remove_group(&self, id: &str) -> Result<()> {
        self.pool
            .run(
                "BEGIN; \
                 DELETE groups:$id; \
                 DELETE FROM group_members WHERE group_id = $id LIMIT ALL; \
                 DELETE FROM group_grants WHERE group_id = $id LIMIT ALL; \
                 COMMIT;",
                vec![text("id", id)],
            )
            .await?;
        Ok(())
    }

    async fn put_member(
        &self,
        group_id: &str,
        member_id: &str,
        user: &UserName,
    ) -> Result<Touched> {
        self.touching(
            "BEGIN; \
             UPDATE groups:$group SET touched = time::now(); \
             UPSERT group_members:$id SET group_id = $group, user_name = $user; \
             COMMIT;",
            vec![
                text("group", group_id),
                text("id", member_id),
                text("user", user.as_str()),
            ],
        )
        .await
    }

    async fn remove_member(&self, member_id: &str) -> Result<bool> {
        let answers = self
            .pool
            .run(
                "DELETE group_members:$id RETURN BEFORE;",
                vec![text("id", member_id)],
            )
            .await?;
        Ok(first_record(answers.into_iter().next())?.is_some())
    }

    async fn members(&self, group_id: &str) -> Result<Vec<MemberEntity>> {
        self.records(
            "SELECT * FROM group_members WHERE group_id = $group USING INDEX by_member_group;",
            vec![text("group", group_id)],
            "group member",
        )
        .await?
        .iter()
        .map(MemberEntity::from_value)
        .collect()
    }

    async fn memberships(&self, user: &UserName) -> Result<Vec<MemberEntity>> {
        self.records(
            "SELECT * FROM group_members WHERE user_name = $user USING INDEX by_member_user;",
            vec![text("user", user.as_str())],
            "group member",
        )
        .await?
        .iter()
        .map(MemberEntity::from_value)
        .collect()
    }

    async fn put_grant(
        &self,
        group_id: &str,
        grant_id: &str,
        bucket: &str,
        access: Access,
    ) -> Result<Touched> {
        self.touching(
            "BEGIN; \
             UPDATE groups:$group SET touched = time::now(); \
             UPSERT group_grants:$id SET group_id = $group, bucket_name = $bucket, \
             can_read = $read, can_write = $write; \
             COMMIT;",
            vec![
                text("group", group_id),
                text("id", grant_id),
                text("bucket", bucket),
                flag("read", access.read),
                flag("write", access.write),
            ],
        )
        .await
    }

    async fn remove_grant(&self, grant_id: &str) -> Result<bool> {
        let answers = self
            .pool
            .run(
                "DELETE group_grants:$id RETURN BEFORE;",
                vec![text("id", grant_id)],
            )
            .await?;
        Ok(first_record(answers.into_iter().next())?.is_some())
    }

    async fn grants(&self, group_id: &str) -> Result<Vec<GroupGrantEntity>> {
        self.records(
            "SELECT * FROM group_grants WHERE group_id = $group USING INDEX by_group_grant;",
            vec![text("group", group_id)],
            "group grant",
        )
        .await?
        .iter()
        .map(GroupGrantEntity::from_value)
        .collect()
    }
}
