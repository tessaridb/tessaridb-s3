//! A bucket as the rest of the server sees it.

use tessari_s3_core::authz::{BucketResource, SpaceName};
use tessari_s3_core::quota::Quota;
use tessari_s3_core::versioning::Versioning;
use tessari_s3_types::{BucketName, Timestamp};

use super::entity::BucketEntity;

/// The stored `acknowledge` of a bucket whose writes its region's majority acknowledges.
pub(crate) const LOCAL: &str = "local";

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
    /// Its versioning state; `None` while it was never versioned.
    pub versioning: Option<Versioning>,
    /// Its lifecycle rules; `None` while none are set.
    pub lifecycle: Option<Vec<tessari_s3_core::lifecycle::Rule>>,
    /// Its notification rules; `None` while none are set.
    pub notification: Option<Vec<tessari_s3_core::notification::Rule>>,
    /// Its replication configuration; `None` while none is set.
    pub replication: Option<tessari_s3_core::replication::Configuration>,
    /// Its media rules; `None` while none are set.
    pub media: Option<Vec<tessari_s3_core::media::Rule>>,
    /// Who must hold an object write before it is acknowledged.
    pub acknowledgement: Acknowledgement,
}

/// Who must hold a bucket's object writes and removals before the client is told they succeeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Acknowledgement {
    /// A majority of the nodes keeping the bucket's region's database — the default; such a write survives a
    /// failover of that database's leadership.
    #[default]
    Majority,
    /// A majority of those nodes inside the region only: faster where the database is also kept in other regions,
    /// and a write acknowledged this way can be LOST to a failover whose new leader did not collect it.
    RegionMajority,
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
        let versioning = entity
            .versioning
            .as_deref()
            .map(|state| {
                Versioning::parse(state).ok_or(Error::Malformed {
                    record: "bucket",
                    reason: "versioning is neither Enabled nor Suspended",
                })
            })
            .transpose()?;
        Ok(Self {
            name,
            created: Timestamp { seconds, nanos },
            region: entity.region,
            incarnation: entity.incarnation,
            space,
            creator: entity.creator,
            quota,
            versioning,
            lifecycle: entity
                .lifecycle
                .as_ref()
                .map(|rules| super::lifecycle::from_values(rules))
                .transpose()?,
            notification: entity
                .notify
                .as_ref()
                .map(super::notification::from_value)
                .transpose()?,
            replication: entity
                .replication
                .as_ref()
                .map(super::replication::from_value)
                .transpose()?,
            media: entity
                .media
                .as_ref()
                .map(|rules| super::media::from_values(rules))
                .transpose()?,
            acknowledgement: match entity.acknowledge.as_deref() {
                None => Acknowledgement::Majority,
                Some(LOCAL) => Acknowledgement::RegionMajority,
                Some(_) => {
                    return Err(Error::Malformed {
                        record: "bucket",
                        reason: "acknowledge is not local",
                    });
                }
            },
        })
    }
}
