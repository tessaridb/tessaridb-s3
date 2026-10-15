//! The usage scanner: measures how many objects and bytes each bucket holds (see `UsageService::refresh`). Every node
//! runs one; the claim in the metadata store lets one of them measure per pass.

use std::time::Duration;

use tessari_s3_constants::DAEMON_RUN_TIMEOUT_SECS;
use tessari_s3_storage::usage::UsageService;
use tessari_s3_types::NodeId;

use crate::Error;
use crate::daemon::{Daemon, DaemonFuture, Report};

/// Measures bucket usage on an interval.
pub struct UsageScanner {
    usage: UsageService,
    holder: String,
    interval: Duration,
}

impl UsageScanner {
    /// A scanner over `usage` running every `interval`, claiming each pass under `node`'s name on a cluster member,
    /// and as `standalone` on a node that is not one.
    #[must_use]
    pub fn new(usage: UsageService, node: Option<&NodeId>, interval: Duration) -> Self {
        Self {
            usage,
            holder: node.map_or_else(|| "standalone".to_owned(), |node| node.as_str().to_owned()),
            interval,
        }
    }
}

impl Daemon for UsageScanner {
    fn name(&self) -> &'static str {
        "usage-scanner"
    }

    fn interval(&self) -> Duration {
        self.interval
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(DAEMON_RUN_TIMEOUT_SECS)
    }

    fn run_once(&self) -> DaemonFuture<'_> {
        Box::pin(async move {
            let measured = self
                .usage
                .refresh(&self.holder)
                .await
                .map_err(|error| Error::Run {
                    daemon: "usage-scanner",
                    message: error.to_string(),
                })?;
            Ok(Report {
                message: measured.map_or_else(
                    || "another member holds the pass".to_owned(),
                    |buckets| format!("measured {buckets} buckets"),
                ),
            })
        })
    }
}
