//! A bucket as the rest of the server sees it.

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
}

impl TryFrom<BucketEntity> for Bucket {
    type Error = Error;

    fn try_from(entity: BucketEntity) -> Result<Self> {
        let name = BucketName::new(&entity.name).map_err(|_| Error::Malformed {
            record: "bucket",
            reason: "name is not a valid bucket name",
        })?;
        let (seconds, nanos) = entity.created;
        Ok(Self {
            name,
            created: Timestamp { seconds, nanos },
            region: entity.region,
            incarnation: entity.incarnation,
        })
    }
}
