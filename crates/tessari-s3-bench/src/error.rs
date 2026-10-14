//! The harness's errors. A configuration error names the variable and never echoes its value.

/// Why the harness could not run or report.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A required variable is unset.
    #[error("{key} is not set")]
    MissingConfig {
        /// The variable.
        key: &'static str,
    },
    /// A variable holds a value the harness cannot use.
    #[error("{key}: {reason}")]
    InvalidConfig {
        /// The variable.
        key: &'static str,
        /// What is wrong with it, without the value.
        reason: &'static str,
    },
    /// The HTTP client could not be built.
    #[error("the HTTP client could not be built")]
    Client,
    /// A request could not be signed.
    #[error("a request could not be signed: {0}")]
    Sign(#[from] tessari_s3_core::auth::AuthError),
    /// The server refused a set-up request (creating the bucket).
    #[error("set-up refused: {status} {code}")]
    SetUp {
        /// The HTTP status.
        status: u16,
        /// The S3 error code.
        code: String,
    },
    /// A request failed below HTTP.
    #[error("transport: {0}")]
    Transport(#[from] reqwest::Error),
    /// A histogram could not be created.
    #[error("histogram: {0}")]
    Histogram(#[from] hdrhistogram::CreationError),
    /// A report could not be read or written.
    #[error("report file: {0}")]
    Io(#[from] std::io::Error),
    /// A report is not the harness's JSON.
    #[error("report format: {0}")]
    Json(#[from] serde_json::Error),
    /// Reports from different set-ups were given to one summary.
    #[error("reports differ in {what}; only like-for-like runs are summarised")]
    Unlike {
        /// The field that differs.
        what: &'static str,
    },
    /// A worker task ended abnormally.
    #[error("a worker task ended abnormally")]
    Task,
}

/// The harness's result.
pub type Result<T> = std::result::Result<T, Error>;
