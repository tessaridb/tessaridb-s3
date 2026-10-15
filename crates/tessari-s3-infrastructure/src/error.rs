//! Failures of `tessari-s3-infrastructure`.

use tessari_s3_types::ErrorCategory;

/// A configuration that cannot start the server.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// A required environment variable is unset or empty.
    #[error("{key} is required and is not set")]
    MissingConfig {
        /// The variable's name.
        key: &'static str,
    },
    /// An environment variable is set to a value the server cannot use. The value is not repeated, because some of
    /// these variables hold secrets.
    #[error("{key} is invalid: {reason}")]
    InvalidConfig {
        /// The variable's name.
        key: &'static str,
        /// What is wrong with it.
        reason: &'static str,
    },
}

impl Error {
    /// The taxonomy category: every configuration failure is `validation`.
    #[must_use]
    pub const fn category(&self) -> ErrorCategory {
        ErrorCategory::Validation
    }
}

/// The result of an operation in this crate.
pub type Result<T> = std::result::Result<T, Error>;
