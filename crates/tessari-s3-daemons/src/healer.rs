//! The healer: on a cluster member, rewrites the missing or damaged shards of the data ids listed for healing (see
//! `ObjectService::heal`).

use std::time::Duration;

use tessari_s3_constants::{DAEMON_RUN_TIMEOUT_SECS, HEAL_BATCH};
use tessari_s3_storage::objects::ObjectService;

use crate::Error;
use crate::daemon::{Daemon, DaemonFuture, Report};

/// Heals listed data ids on an interval.
pub struct Healer {
    objects: ObjectService,
    interval: Duration,
}

impl Healer {
    /// A healer over `objects`, running every `interval`.
    #[must_use]
    pub const fn new(objects: ObjectService, interval: Duration) -> Self {
        Self { objects, interval }
    }
}

impl Daemon for Healer {
    fn name(&self) -> &'static str {
        "healer"
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
                .heal(HEAL_BATCH)
                .await
                .map_err(|error| Error::Run {
                    daemon: "healer",
                    message: error.to_string(),
                })?;
            Ok(Report {
                message: format!(
                    "examined {}, healed {}, at risk {}, held elsewhere {}",
                    done.examined, done.healed, done.at_risk, done.held
                ),
            })
        })
    }
}
