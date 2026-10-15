//! Runs a daemon on its interval until shutdown.

use std::sync::Arc;

use tokio::sync::watch;

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

/// Runs `daemon` every interval until `shutdown` changes or its sender is dropped. A run that is under way when
/// shutdown arrives finishes or reaches its timeout first; no new run starts after it.
pub async fn run(daemon: Arc<dyn Daemon>, mut shutdown: watch::Receiver<bool>) -> Runs {
    let name = daemon.name();
    let timeout = daemon.timeout();
    let mut ticker = tokio::time::interval(daemon.interval());
    // A run that overran its interval does not trigger a burst of catch-up runs.
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut runs = Runs::default();
    loop {
        tokio::select! {
            biased;
            _ = shutdown.changed() => {
                tracing::info!(daemon = name, ?runs, "daemon stopping");
                return runs;
            }
            _ = ticker.tick() => {
                match tokio::time::timeout(timeout, daemon.run_once()).await {
                    Ok(Ok(report)) => {
                        runs.succeeded = runs.succeeded.saturating_add(1);
                        tracing::info!(daemon = name, result = %report.message, "daemon run completed");
                    }
                    Ok(Err(error)) => {
                        runs.failed = runs.failed.saturating_add(1);
                        tracing::error!(daemon = name, error = %error, "daemon run failed");
                    }
                    Err(_) => {
                        runs.timed_out = runs.timed_out.saturating_add(1);
                        tracing::error!(daemon = name, timeout_secs = timeout.as_secs(), "daemon run timed out");
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "runner_tests.rs"]
mod tests;
