//! Groups as the rest of the server sees them, and what changing one answered.

use tessari_s3_core::authz::{Access, GroupName, SpaceName};
use tessari_s3_types::Timestamp;

use super::entity::{GroupEntity, GroupGrantEntity};
use crate::{Error, Result};

/// One group of a space.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    /// The space it belongs to.
    pub space: SpaceName,
    /// Its name there.
    pub name: GroupName,
    /// When it was created.
    pub created: Timestamp,
}

/// One bucket a group's members receive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupGrant {
    /// The bucket, by name.
    pub bucket: String,
    /// What it allows.
    pub access: Access,
}

/// What adding a member or a grant to a group ended in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberChange {
    /// Written.
    Done,
    /// There is no such group (or it was removed while the change was written).
    NoSuchGroup,
    /// The user or bucket belongs to another space; nothing was written.
    OtherSpace,
}

impl TryFrom<GroupEntity> for Group {
    type Error = Error;

    fn try_from(entity: GroupEntity) -> Result<Self> {
        let malformed = |reason| Error::Malformed {
            record: "group",
            reason,
        };
        let (seconds, nanos) = entity.created;
        Ok(Self {
            space: SpaceName::new(&entity.space_name).ok_or(malformed("space_name"))?,
            name: GroupName::new(&entity.name).ok_or(malformed("name"))?,
            created: Timestamp { seconds, nanos },
        })
    }
}

impl From<GroupGrantEntity> for GroupGrant {
    fn from(entity: GroupGrantEntity) -> Self {
        Self {
            bucket: entity.bucket_name,
            access: Access {
                read: entity.can_read,
                write: entity.can_write,
            },
        }
    }
}
