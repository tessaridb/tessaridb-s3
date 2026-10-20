//! The re-wrapper: after a rotation, re-wraps the data keys still wrapped under an older server key (see
//! `ObjectService::rewrap`). Every node runs one; a record two of them reach at once is re-wrapped by one, because
//! each re-wrap is a compare-and-set on the key it read. A node whose keys are sealed has nothing to do.

use std::time::Duration;

use tessari_s3_constants::{DAEMON_RUN_TIMEOUT_SECS, REWRAP_BATCH, REWRAP_INTERVAL_SECS};
use tessari_s3_storage::objects::ObjectService;

use crate::Error;
use crate::daemon::{Daemon, DaemonFuture, Report};

/// Re-wraps data keys under the active server key on an interval.
pub struct Rewrapper {
    objects: ObjectService,
}

impl Rewrapper {
    /// A re-wrapper over `objects`.
    #[must_use]
    pub const fn new(objects: ObjectService) -> Self {
        Self { objects }
    }
}

impl Daemon for Rewrapper {
    fn name(&self) -> &'static str {
        "rewrapper"
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(REWRAP_INTERVAL_SECS)
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(DAEMON_RUN_TIMEOUT_SECS)
    }

    fn run_once(&self) -> DaemonFuture<'_> {
        Box::pin(async move {
            match self.objects.rewrap(REWRAP_BATCH).await {
                Ok(done) => Ok(Report {
                    message: format!("re-wrapped {} of {}", done.rewrapped, done.examined),
                    idle: done.examined == 0,
                }),
                Err(tessari_s3_storage::Error::KeysSealed) => Ok(Report {
                    message: "the keys are sealed; nothing re-wrapped".to_owned(),
                    // Sealed keys are reported at start and in the console; every run saying so again is noise.
                    idle: true,
                }),
                Err(error) => Err(Error::Run {
                    daemon: "rewrapper",
                    message: error.to_string(),
                }),
            }
        })
    }
}
