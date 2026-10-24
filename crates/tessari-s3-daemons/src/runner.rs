//! Runs a daemon on its interval until shutdown.

use std::sync::Arc;
use std::time::Duration;

use tessari_s3_observability::{Metrics, RunOutcome};
use tokio::sync::watch;
use tokio::time::Instant;

use crate::daemon::Daemon;

/// What a runner did before it returned, for tests and the final log line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Runs {
    /// Runs that reported success.
    pub succeeded: u64,
    /// Runs that failed.
    pub failed: u64,
    /// Runs abandoned at their timeout.
    pub timed_out: u64,
}

/// Runs `daemon` at once and then every interval — read again after each run — until `shutdown` changes or its sender
/// is dropped, counting each run by its outcome in `metrics`. A run that is under way when shutdown arrives finishes or
/// reaches its timeout first; no new run starts after it.
pub async fn run(
    daemon: Arc<dyn Daemon>,
    mut shutdown: watch::Receiver<bool>,
    metrics: Arc<Metrics>,
) -> Runs {
    let name = daemon.name();
    let timeout = daemon.timeout();
    let mut next = Instant::now();
    let mut runs = Runs::default();
    loop {
        tokio::select! {
            biased;
            _ = shutdown.changed() => {
                tracing::info!(daemon = name, ?runs, "daemon stopping");
                return runs;
            }
            () = tokio::time::sleep_until(next) => {
                let started = Instant::now();
                match tokio::time::timeout(timeout, daemon.run_once()).await {
                    Ok(Ok(report)) => {
                        runs.succeeded = runs.succeeded.saturating_add(1);
                        metrics.state().run(name, RunOutcome::Succeeded);
                        if report.idle {
                            tracing::debug!(daemon = name, result = %report.message, "daemon run completed");
                        } else {
                            tracing::info!(daemon = name, result = %report.message, "daemon run completed");
                        }
                    }
                    Ok(Err(error)) => {
                        runs.failed = runs.failed.saturating_add(1);
                        metrics.state().run(name, RunOutcome::Failed);
                        tracing::error!(daemon = name, error = %error, "daemon run failed");
                    }
                    Err(_) => {
                        runs.timed_out = runs.timed_out.saturating_add(1);
                        metrics.state().run(name, RunOutcome::TimedOut);
                        tracing::error!(daemon = name, timeout_secs = timeout.as_secs(), "daemon run timed out");
                    }
                }
                next = next_start(started, daemon.interval());
            }
        }
    }
}

/// When the run after one that started at `started` begins: its interval later, read after every run so a changed
/// setting paces the next wait; a run that overran its interval waits a whole interval from now rather than starting
/// a burst of catch-up runs.
fn next_start(started: Instant, interval: Duration) -> Instant {
    let now = Instant::now();
    started
        .checked_add(interval)
        .filter(|due| *due > now)
        .or_else(|| now.checked_add(interval))
        // An interval past what the clock can count: wait as long as it can, which no configured value reaches.
        .or_else(|| now.checked_add(Duration::from_secs(u64::from(u32::MAX))))
        .unwrap_or(now)
}

#[cfg(test)]
#[path = "runner_tests.rs"]
mod tests;
