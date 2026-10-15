//! A daemon: periodic work the server does on its own, independent of requests.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use crate::Result;

/// What one run reports, for the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// One line describing what the run did.
    pub message: String,
}

/// The future one run returns.
pub type DaemonFuture<'a> = Pin<Box<dyn Future<Output = Result<Report>> + Send + 'a>>;

/// Periodic background work with a name, a cadence and a bound on each run.
pub trait Daemon: Send + Sync {
    /// The name every log line carries.
    fn name(&self) -> &'static str;
    /// Time between runs.
    fn interval(&self) -> Duration;
    /// Longest one run may take before it is abandoned and reported.
    fn timeout(&self) -> Duration;
    /// One run.
    fn run_once(&self) -> DaemonFuture<'_>;
}
