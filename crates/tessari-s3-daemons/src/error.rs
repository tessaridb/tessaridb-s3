//! Failures of `tessari-s3-daemons`.

/// A daemon run that did not complete.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The run failed; the daemon runs again at its next tick.
    #[error("{daemon} failed: {message}")]
    Run {
        /// Which daemon.
        daemon: &'static str,
        /// What went wrong, already free of secrets.
        message: String,
    },
}

/// The result of an operation in this crate.
pub type Result<T> = std::result::Result<T, Error>;
