//! Failures of `tessari-s3-observability`.

/// Everything that can go wrong in this crate.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The metrics could not be written as text.
    #[error("the metrics could not be encoded")]
    Encode,
}

/// The result of an operation in this crate.
pub type Result<T> = std::result::Result<T, Error>;
