//! What the lifecycle queue must answer, whatever stores it.

use std::future::Future;

use super::entity::RunEntity;
use crate::Result;

/// Lifecycle-queue queries: one record per bucket with rules, held by one node at a time.
pub(crate) trait LifecycleRepository: Send + Sync {
    /// Claims the next bucket due, if any.
    fn claim(&self) -> impl Future<Output = Result<Option<RunEntity>>> + Send;
    /// Hands `bucket` back, due again `interval_secs` from the store's now; a record removed meanwhile is left gone.
    fn release(&self, bucket: &str, interval_secs: u64) -> impl Future<Output = Result<()>> + Send;
    /// Removes `run`'s record while it is still the one claimed; `false` when the rules were written again since.
    fn forget(&self, run: &RunEntity) -> impl Future<Output = Result<bool>> + Send;
}
