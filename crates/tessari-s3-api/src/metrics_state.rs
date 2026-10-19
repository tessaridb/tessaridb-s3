//! The node's state read at a scrape: a probe of the metadata store, the keys vault, the data drive, the latest usage
//! measurement and the open uploads. Each read has its own deadline and says in `collect_up` whether it answered; a
//! read that did not keeps its series at their last values.

use std::future::Future;
use std::time::Duration;

use tessari_s3_constants::observability::{METRICS_BACKLOG_COUNT_MAX, METRICS_COLLECT_SECS};
use tessari_s3_observability::{Counted, Metrics, VaultState};
use tessari_s3_storage::keys::KeyState;
use tokio::time::timeout;

use crate::ApiState;

/// What `read` answered within the scrape's deadline, recorded under `source` in `collect_up`; `None` otherwise.
async fn read<T>(
    metrics: &Metrics,
    source: &'static str,
    read: impl Future<Output = tessari_s3_storage::Result<T>>,
) -> Option<T> {
    match timeout(Duration::from_secs(METRICS_COLLECT_SECS), read).await {
        Ok(Ok(value)) => {
            metrics.collected(source, true);
            Some(value)
        }
        Ok(Err(error)) => {
            tracing::warn!(source, error = %error, "a metrics source could not be read");
            metrics.collected(source, false);
            None
        }
        Err(_) => {
            tracing::warn!(source, "a metrics source was not read in time");
            metrics.collected(source, false);
            None
        }
    }
}

/// Reads the node's state into its gauges.
pub(crate) async fn collect(state: &ApiState) {
    let metrics = state.metrics();
    let storage = state.storage();
    let series = metrics.state();
    if let Some(took) = read(metrics, "meta", storage.probe()).await {
        series.meta_probe(took.as_secs_f64());
    }
    if let Some(keys) = read(metrics, "keys", storage.keys().state()).await {
        series.vault(match keys {
            KeyState::Unsealed { .. } => VaultState::Unsealed,
            KeyState::Sealed => VaultState::Sealed,
            KeyState::Uninitialised => VaultState::Uninitialised,
        });
    }
    if let Some(drive) = read(metrics, "drive", storage.shards().drive()).await {
        series.data_space(drive.capacity, drive.available);
    }
    if let Some(Some(usage)) = read(metrics, "usage", storage.usage().latest()).await {
        let sum = |field: fn(&tessari_s3_storage::usage::BucketUsage) -> u64| {
            usage
                .buckets
                .iter()
                .map(field)
                .fold(0_u64, u64::saturating_add)
        };
        series.usage(Counted {
            buckets: u64::try_from(usage.buckets.len()).unwrap_or(u64::MAX),
            objects: sum(|bucket| bucket.objects),
            bytes: sum(|bucket| bucket.bytes),
            taken_secs: usage.taken.seconds,
        });
    }
    let uploads = storage.multipart().open_count(METRICS_BACKLOG_COUNT_MAX);
    if let Some(open) = read(metrics, "uploads", uploads).await {
        series.uploads(u64::try_from(open).unwrap_or(u64::MAX));
    }
}
