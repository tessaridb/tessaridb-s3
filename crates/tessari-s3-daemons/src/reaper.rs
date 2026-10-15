//! The upload reaper: aborts multipart uploads open longer than the maximum age (see `MultipartService::reap`), the
//! server-wide backstop for uploads nobody completes or aborts. Their parts' files are queued for the reclaimer by the
//! abort itself.

use std::time::Duration;

use tessari_s3_constants::{DAEMON_RUN_TIMEOUT_SECS, REAP_BATCH};
use tessari_s3_storage::multipart::MultipartService;

use crate::Error;
use crate::daemon::{Daemon, DaemonFuture, Report};

/// Aborts uploads past the maximum age on an interval.
pub struct UploadReaper {
    multipart: MultipartService,
    max_age_secs: u64,
    interval: Duration,
}

impl UploadReaper {
    /// A reaper over `multipart`, aborting uploads older than `max_age_secs` and running every `interval`.
    #[must_use]
    pub const fn new(multipart: MultipartService, max_age_secs: u64, interval: Duration) -> Self {
        Self {
            multipart,
            max_age_secs,
            interval,
        }
    }
}

impl Daemon for UploadReaper {
    fn name(&self) -> &'static str {
        "upload-reaper"
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
                .multipart
                .reap(self.max_age_secs, REAP_BATCH)
                .await
                .map_err(|error| Error::Run {
                    daemon: "upload-reaper",
                    message: error.to_string(),
                })?;
            Ok(Report {
                message: format!("examined {}, aborted {}", done.examined, done.aborted),
            })
        })
    }
}
