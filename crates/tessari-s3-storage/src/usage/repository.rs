//! What usage persistence must answer, whatever stores it.

use std::future::Future;

use super::entity::{BucketUsageEntity, UsageEntity};
use super::model::BucketCount;
use super::occupancy::Measured;
use crate::Result;

/// Usage queries; no rule beyond what a query states.
pub(crate) trait UsageRepository: Send + Sync {
    /// Takes the measuring claim for `holder` — free, or already `holder`'s — for the claim's lifetime.
    fn claim(&self, holder: &str) -> impl Future<Output = Result<bool>> + Send;
    /// What every bucket holds, read now: objects folded per bucket, parts per upload, and the uploads' buckets.
    fn measure(&self) -> impl Future<Output = Result<Measured>> + Send;
    /// Replaces the stored measurement with `buckets`, stamped with the store's clock.
    fn store(&self, buckets: &[BucketUsageEntity]) -> impl Future<Output = Result<()>> + Send;
    /// What `bucket` holds now, read through its index within the quota count's time limit.
    fn count_bucket(&self, bucket: &str) -> impl Future<Output = Result<BucketCount>> + Send;
    /// The stored measurement, if one was ever taken.
    fn latest(&self) -> impl Future<Output = Result<Option<UsageEntity>>> + Send;
}
