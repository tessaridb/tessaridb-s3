//! The lifecycle worker: applies the rules of the buckets the `lifecycle_runs` queue hands out (see
//! `LifecycleService::pass`), at this node's clock.

use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tessari_s3_infrastructure::Live;

use tessari_s3_constants::{DAEMON_RUN_TIMEOUT_SECS, LIFECYCLE_BATCH, LIFECYCLE_INTERVAL_SECS};
use tessari_s3_storage::lifecycle::LifecycleService;
use tessari_s3_types::Timestamp;

use crate::Error;
use crate::daemon::{Daemon, DaemonFuture, Report};

/// Applies buckets' lifecycle rules on an interval.
pub struct LifecycleWorker {
    lifecycle: LifecycleService,
    interval: Live,
}

impl LifecycleWorker {
    /// A worker over `lifecycle`, looking for due buckets every `interval`.
    #[must_use]
    pub const fn new(lifecycle: LifecycleService, interval: Live) -> Self {
        Self {
            lifecycle,
            interval,
        }
    }
}

/// This node's clock as the rules read it.
fn now() -> Result<Timestamp, Error> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| Error::Run {
            daemon: "lifecycle",
            message: error.to_string(),
        })?;
    let seconds = i64::try_from(elapsed.as_secs()).map_err(|error| Error::Run {
        daemon: "lifecycle",
        message: error.to_string(),
    })?;
    Ok(Timestamp {
        seconds,
        nanos: elapsed.subsec_nanos(),
    })
}

impl Daemon for LifecycleWorker {
    fn name(&self) -> &'static str {
        "lifecycle"
    }

    fn interval(&self) -> Duration {
        self.interval.secs()
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(DAEMON_RUN_TIMEOUT_SECS)
    }

    fn run_once(&self) -> DaemonFuture<'_> {
        Box::pin(async move {
            let done = self
                .lifecycle
                .pass(now()?, LIFECYCLE_BATCH, LIFECYCLE_INTERVAL_SECS)
                .await
                .map_err(|error| Error::Run {
                    daemon: "lifecycle",
                    message: error.to_string(),
                })?;
            Ok(Report {
                message: format!(
                    "buckets {}, expired {}, noncurrent {}, markers {}, aborted {}",
                    done.buckets, done.expired, done.noncurrent, done.markers, done.aborted
                ),
                idle: done.buckets == 0,
            })
        })
    }
}
