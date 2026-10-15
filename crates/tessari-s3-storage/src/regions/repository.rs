//! What failover persistence must answer, whatever stores it.

use std::future::Future;

use super::entity::RegionEntity;
use crate::Result;

/// The fields a failover or a restore writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Change<'a> {
    /// The region.
    pub(crate) region: &'a str,
    /// Its new epoch.
    pub(crate) epoch: i64,
    /// Where its writes go; `None` restores it.
    pub(crate) promoted: Option<&'a str>,
    /// The operator's reason.
    pub(crate) reason: &'a str,
    /// Who.
    pub(crate) by: &'a str,
}

/// Failover queries; no rule beyond what a query states. A record is never deleted.
pub(crate) trait RegionRepository: Send + Sync {
    /// The record of `region`, if any.
    fn get(&self, region: &str) -> impl Future<Output = Result<Option<RegionEntity>>> + Send;
    /// Every record in region order, at most the server's cap.
    fn list(&self) -> impl Future<Output = Result<Vec<RegionEntity>>> + Send;
    /// Writes `change` only when the region has no record; `false` when one exists, and then nothing is written.
    fn create(&self, change: &Change<'_>) -> impl Future<Output = Result<bool>> + Send;
    /// Writes `change` only while the record's epoch is still `seen`; `false` otherwise, and nothing is written.
    fn replace(&self, change: &Change<'_>, seen: i64) -> impl Future<Output = Result<bool>> + Send;
}
