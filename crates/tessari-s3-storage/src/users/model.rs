//! Users as the rest of the server sees them, what creating one takes, and what issuing or resolving a key answers.

use tessari_s3_core::authz::{Role, SpaceName, UserName};
use tessari_s3_types::{SecretKey, Timestamp};

use super::entity::UserEntity;
use crate::{Error, Result};

/// How a role is stored.
pub(crate) const fn role_text(role: Role) -> &'static str {
    match role {
        Role::SpaceAdmin => "admin",
        Role::Member => "member",
    }
}

/// One user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    /// Its name.
    pub name: UserName,
    /// The space it belongs to.
    pub space: SpaceName,
    /// Its role in that space.
    pub role: Role,
    /// Whether a member may create buckets in the space.
    pub create_buckets: bool,
    /// Store-wide: every space, users, quotas.
    pub operator: bool,
    /// Store-wide: the cluster's members, drives and healing.
    pub cluster_viewer: bool,
    /// A disabled user's keys resolve to nothing.
    pub disabled: bool,
    /// When it was created.
    pub created: Timestamp,
}

/// What creating a user takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewUser {
    /// Its name, unique across the store.
    pub name: UserName,
    /// The space it joins; it must exist.
    pub space: SpaceName,
    /// Its role there.
    pub role: Role,
    /// Whether a member may create buckets in the space.
    pub create_buckets: bool,
    /// Store-wide operator.
    pub operator: bool,
    /// Store-wide cluster viewer.
    pub cluster_viewer: bool,
}

/// A key just issued: the only time its secret is shown. Not `Clone`: every copy of the secret is a deliberate act.
#[derive(Debug)]
pub struct IssuedKey {
    /// The access key id, `TS` and 18 more characters.
    pub access_key_id: String,
    /// The secret, 40 characters; stored only sealed.
    pub secret: SecretKey,
}

/// A live key's user and secret, for verifying a request it signed. Not `Clone`, for the same reason.
#[derive(Debug)]
pub struct Resolved {
    /// The user holding the key.
    pub user: User,
    /// The key's secret.
    pub secret: SecretKey,
}

impl TryFrom<UserEntity> for User {
    type Error = Error;

    fn try_from(entity: UserEntity) -> Result<Self> {
        let malformed = |reason| Error::Malformed {
            record: "user",
            reason,
        };
        let name = UserName::new(&entity.name).ok_or(malformed("name is not a valid user name"))?;
        let space = SpaceName::new(&entity.space_name)
            .ok_or(malformed("space_name is not a valid space name"))?;
        let role = match entity.space_role.as_str() {
            "admin" => Role::SpaceAdmin,
            "member" => Role::Member,
            _ => return Err(malformed("space_role")),
        };
        let (seconds, nanos) = entity.created;
        Ok(Self {
            name,
            space,
            role,
            create_buckets: entity.create_buckets,
            operator: entity.operator,
            cluster_viewer: entity.cluster_viewer,
            disabled: entity.disabled,
            created: Timestamp { seconds, nanos },
        })
    }
}
