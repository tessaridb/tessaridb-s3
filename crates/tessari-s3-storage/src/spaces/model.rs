//! A space as the rest of the server sees it.

use tessari_s3_core::authz::SpaceName;
use tessari_s3_types::Timestamp;

use super::entity::SpaceEntity;
use crate::{Error, Result};

/// One space.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Space {
    /// Its name.
    pub name: SpaceName,
    /// When it was created.
    pub created: Timestamp,
}

impl TryFrom<SpaceEntity> for Space {
    type Error = Error;

    fn try_from(entity: SpaceEntity) -> Result<Self> {
        let name = SpaceName::new(&entity.name).ok_or(Error::Malformed {
            record: "space",
            reason: "name is not a valid space name",
        })?;
        let (seconds, nanos) = entity.created;
        Ok(Self {
            name,
            created: Timestamp { seconds, nanos },
        })
    }
}
