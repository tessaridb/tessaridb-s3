//! The mover: on a cluster member, moves data written under an older layout onto the current one and retires what
//! earlier moves left behind (see `ObjectService::move_layouts`), paced to a budget of bytes a second.

use std::future::Future;
use std::num::NonZeroU64;
use std::time::Duration;
use tessari_s3_infrastructure::Live;

use tessari_s3_constants::MOVE_BATCH;
use tessari_s3_storage::objects::ObjectService;
use tokio::sync::watch;

use crate::Error;
use crate::daemon::{Daemon, DaemonFuture, Report};

/// Moves data onto the current layout on an interval.
pub struct Mover {
    objects: ObjectService,
    interval: Live,
    grace_secs: u64,
    bytes_per_sec: Live,
    shutdown: watch::Receiver<bool>,
}

impl Mover {
    /// A mover over `objects`, running every `interval`, retiring old shards `grace_secs` after a move, copying at
    /// most `bytes_per_sec` (`0` pauses it), and stopping a run when `shutdown` turns true.
    #[must_use]
    pub const fn new(
        objects: ObjectService,
        interval: Live,
        grace_secs: u64,
        bytes_per_sec: Live,
        shutdown: watch::Receiver<bool>,
    ) -> Self {
        Self {
            objects,
            interval,
            grace_secs,
            bytes_per_sec,
            shutdown,
        }
    }
}

/// `work`, unless `shutdown` turns true (or its sender goes) first: `None` then.
async fn until_stopped<T>(
    mut shutdown: watch::Receiver<bool>,
    work: impl Future<Output = T>,
) -> Option<T> {
    tokio::select! {
        // Shutdown first: a run asked to stop does not start, or go on with, the work.
        biased;
        _ = shutdown.wait_for(|stopped| *stopped) => None,
        done = work => Some(done),
    }
}

impl Daemon for Mover {
    fn name(&self) -> &'static str {
        "mover"
    }

    fn interval(&self) -> Duration {
        self.interval.secs()
    }

    /// None: a paced move of a large object takes as long as its bytes over the budget, so a run is bounded by its
    /// own shutdown watch and, per block, by the shard stall — never by a clock that would cut every attempt short.
    fn timeout(&self) -> Duration {
        Duration::MAX
    }

    fn run_once(&self) -> DaemonFuture<'_> {
        Box::pin(async move {
            let Some(budget) = NonZeroU64::new(self.bytes_per_sec.get()) else {
                return Ok(Report {
                    message: "paused".to_owned(),
                    idle: true,
                });
            };
            let pass = self
                .objects
                .move_layouts(self.grace_secs, MOVE_BATCH, budget);
            let Some(done) = until_stopped(self.shutdown.clone(), pass).await else {
                return Ok(Report {
                    message: "stopped for shutdown; the pass resumes from the store".to_owned(),
                    idle: false,
                });
            };
            let done = done.map_err(|error| Error::Run {
                daemon: "mover",
                message: error.to_string(),
            })?;
            Ok(Report {
                message: format!(
                    "examined {}, moved {}, retired {}, held elsewhere {}",
                    done.examined, done.moved, done.retired, done.held
                ),
                idle: done.examined == 0,
            })
        })
    }
}

#[cfg(test)]
#[path = "mover_tests.rs"]
mod tests;
