//! A bucket as the rest of the server sees it.

use tessari_s3_core::authz::{BucketResource, SpaceName};
use tessari_s3_core::quota::Quota;
use tessari_s3_types::{BucketName, Timestamp};

use super::entity::BucketEntity;

use crate::{Error, Result};

/// One bucket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bucket {
    /// Its name.
    pub name: BucketName,
    /// When it was created.
    pub created: Timestamp,
    /// The region it lives in.
    pub region: String,
    /// Which creation of this name it is.
    pub incarnation: [u8; 16],
    /// The space that owns it.
    pub space: SpaceName,
    /// The user who created it; `None` for the root credential.
    pub creator: Option<String>,
    /// Its limits, set by an operator; none by default.
    pub quota: Quota,
}

impl Bucket {
    /// The bucket as the access evaluator sees it.
    #[must_use]
    pub fn resource(&self) -> BucketResource {
        BucketResource {
            name: self.name.as_str().to_owned(),
            space: self.space.clone(),
            creator: self.creator.clone(),
        }
    }
}

impl TryFrom<BucketEntity> for Bucket {
    type Error = Error;

    fn try_from(entity: BucketEntity) -> Result<Self> {
        let name = BucketName::new(&entity.name).map_err(|_| Error::Malformed {
            record: "bucket",
            reason: "name is not a valid bucket name",
        })?;
        let space = SpaceName::new(&entity.space_name).ok_or(Error::Malformed {
            record: "bucket",
            reason: "space_name is not a valid space name",
        })?;
        let (seconds, nanos) = entity.created;
        let limit = |stored: Option<i64>| {
            stored
                .map(u64::try_from)
                .transpose()
                .map_err(|_| Error::Malformed {
                    record: "bucket",
                    reason: "a quota limit is negative",
                })
        };
        let quota = Quota {
            max_bytes: limit(entity.max_bytes)?,
            max_objects: limit(entity.max_objects)?,
        };
        Ok(Self {
            name,
            created: Timestamp { seconds, nanos },
            region: entity.region,
            incarnation: entity.incarnation,
            space,
            creator: entity.creator,
            quota,
        })
    }
}
