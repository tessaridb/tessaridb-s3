//! Failover rules: a region is failed over to another region, never to itself and never twice, and restored only
//! while failed over; each change takes the next epoch by compare-and-set, so two operators acting at once cannot both
//! succeed.

use tessari_s3_types::Timestamp;

use super::entity::RegionEntity;
use super::model::{FailOver, RegionState, Restore};
use super::repository::{Change, RegionRepository};
use super::tessaridb::TessariRegions;
use crate::{Error, Result};

/// Region failover and restore.
#[derive(Clone)]
pub struct RegionService {
    repository: TessariRegions,
}

fn state(entity: RegionEntity) -> Result<RegionState> {
    let (seconds, nanos) = entity.at;
    Ok(RegionState {
        region: entity.region,
        epoch: u64::try_from(entity.epoch).map_err(|_| Error::Malformed {
            record: "region failover",
            reason: "a negative epoch",
        })?,
        promoted: entity.promoted,
        reason: entity.reason,
        by: entity.by,
        at: Timestamp { seconds, nanos },
    })
}

impl RegionService {
    /// The service over `repository`.
    #[must_use]
    pub const fn new(repository: TessariRegions) -> Self {
        Self { repository }
    }

    /// The record of `region`; `None` for a region never failed over.
    ///
    /// # Errors
    /// The metadata store's refusal or outage, or a malformed record.
    pub async fn state(&self, region: &str) -> Result<Option<RegionState>> {
        self.repository.get(region).await?.map(state).transpose()
    }

    /// The record of `region` while it is failed over; `None` while it takes client writes.
    ///
    /// # Errors
    /// The metadata store's refusal or outage, or a malformed record.
    pub async fn failed_over(&self, region: &str) -> Result<Option<RegionState>> {
        Ok(self.state(region).await?.filter(RegionState::failed_over))
    }

    /// Every region ever failed over, in region order.
    ///
    /// # Errors
    /// The metadata store's refusal or outage, or a malformed record.
    pub async fn list(&self) -> Result<Vec<RegionState>> {
        self.repository
            .list()
            .await?
            .into_iter()
            .map(state)
            .collect()
    }

    /// Fails `region` over to `promoted` under the next epoch.
    ///
    /// # Errors
    /// The metadata store's refusal or outage, or a malformed record.
    pub async fn fail_over(
        &self,
        region: &str,
        promoted: &str,
        by: &str,
        reason: &str,
    ) -> Result<FailOver> {
        if region == promoted {
            return Ok(FailOver::ToItself);
        }
        let current = self.repository.get(region).await?;
        let seen = match &current {
            Some(entity) if entity.promoted.is_some() => {
                return Ok(FailOver::AlreadyFailedOver(state(entity.clone())?));
            }
            Some(entity) => Some(entity.epoch),
            None => None,
        };
        let change = Change {
            region,
            epoch: seen.unwrap_or(0).checked_add(1).ok_or(Error::Malformed {
                record: "region failover",
                reason: "the epoch is exhausted",
            })?,
            promoted: Some(promoted),
            reason,
            by,
        };
        let landed = match seen {
            None => self.repository.create(&change).await?,
            Some(seen) => self.repository.replace(&change, seen).await?,
        };
        if !landed {
            return Ok(FailOver::Contended);
        }
        self.written(region).await.map(FailOver::FailedOver)
    }

    /// Opens `region` to client writes again under the next epoch.
    ///
    /// # Errors
    /// The metadata store's refusal or outage, or a malformed record.
    pub async fn restore(&self, region: &str, by: &str, reason: &str) -> Result<Restore> {
        let Some(current) = self.repository.get(region).await? else {
            return Ok(Restore::NotFailedOver);
        };
        if current.promoted.is_none() {
            return Ok(Restore::NotFailedOver);
        }
        let change = Change {
            region,
            epoch: current.epoch.checked_add(1).ok_or(Error::Malformed {
                record: "region failover",
                reason: "the epoch is exhausted",
            })?,
            promoted: None,
            reason,
            by,
        };
        if !self.repository.replace(&change, current.epoch).await? {
            return Ok(Restore::Contended);
        }
        self.written(region).await.map(Restore::Restored)
    }

    /// The record just written, read back.
    async fn written(&self, region: &str) -> Result<RegionState> {
        self.repository
            .get(region)
            .await?
            .map(state)
            .transpose()?
            .ok_or(Error::Malformed {
                record: "region failover",
                reason: "a written record is missing",
            })
    }
}
