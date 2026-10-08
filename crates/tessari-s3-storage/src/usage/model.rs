//! Usage as the rest of the server reads it.

use tessari_s3_types::Timestamp;

use super::entity::{BucketUsageEntity, UsageEntity};
use crate::{Error, Result};

/// One bucket's figures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BucketUsage {
    /// The bucket's name.
    pub bucket: String,
    /// Objects it holds.
    pub objects: u64,
    /// The objects' logical size in bytes — what clients stored, before erasure coding.
    pub bytes: u64,
}

/// The latest measurement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Usage {
    /// When it was taken, by the metadata store's clock.
    pub taken: Timestamp,
    /// Every bucket holding at least one object, by name; a bucket absent here holds none.
    pub buckets: Vec<BucketUsage>,
}

impl TryFrom<BucketUsageEntity> for BucketUsage {
    type Error = Error;

    fn try_from(entity: BucketUsageEntity) -> Result<Self> {
        let count = |number: i64, reason| {
            u64::try_from(number).map_err(|_| Error::Malformed {
                record: "bucket_usage",
                reason,
            })
        };
        Ok(Self {
            objects: count(entity.objects, "objects")?,
            bytes: count(entity.bytes, "bytes")?,
            bucket: entity.bucket,
        })
    }
}

impl TryFrom<UsageEntity> for Usage {
    type Error = Error;

    fn try_from(entity: UsageEntity) -> Result<Self> {
        let (seconds, nanos) = entity.taken;
        let mut buckets = entity
            .buckets
            .into_iter()
            .map(BucketUsage::try_from)
            .collect::<Result<Vec<_>>>()?;
        buckets.sort_by(|left, right| left.bucket.cmp(&right.bucket));
        Ok(Self {
            taken: Timestamp { seconds, nanos },
            buckets,
        })
    }
}
