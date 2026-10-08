//! The reclaimer: removes data files the `gc` queue names once they are past the grace period (see
//! `ObjectService::reclaim` for what makes that safe against a racing commit).

use std::time::Duration;

use tessari_s3_constants::{DAEMON_RUN_TIMEOUT_SECS, RECLAIM_BATCH};
use tessari_s3_storage::objects::ObjectService;

use crate::Error;
use crate::daemon::{Daemon, DaemonFuture, Report};

/// Reclaims queued data files on an interval.
pub struct Reclaimer {
    objects: ObjectService,
    grace_secs: u64,
    interval: Duration,
}

impl Reclaimer {
    /// A reclaimer over `objects`, keeping queued files for `grace_secs` and running every `interval`.
    #[must_use]
    pub const fn new(objects: ObjectService, grace_secs: u64, interval: Duration) -> Self {
        Self {
            objects,
            grace_secs,
            interval,
        }
    }
}

impl Daemon for Reclaimer {
    fn name(&self) -> &'static str {
        "reclaimer"
    }

    fn interval(&self) -> Duration {
        self.interval
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(DAEMON_RUN_TIMEOUT_SECS)
    }

    fn run_once(&self) -> DaemonFuture<'_> {
        Box::pin(async move {
            let done = self
                .objects
                .reclaim(self.grace_secs, RECLAIM_BATCH)
                .await
                .map_err(|error| Error::Run {
                    daemon: "reclaimer",
                    message: error.to_string(),
                })?;
            Ok(Report {
                message: format!(
                    "examined {}, removed {}, kept {}, orphans {}",
                    done.examined, done.removed, done.kept, done.orphans
                ),
            })
        })
    }
}
