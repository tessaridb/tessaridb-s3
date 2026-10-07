//! Failures of `tessari-s3-types`.
//!
//! Every variant is a `validation` failure in the workspace's error taxonomy: a value from outside did not satisfy
//! the published S3 limits. [`Error::category`] names it so callers route on the category, never on the message.

use crate::ErrorCategory;

/// A value that does not satisfy the S3 rule it was checked against.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The bucket name breaks the AWS general purpose bucket naming rules; the S3 answer is `InvalidBucketName`.
    #[error("invalid bucket name: {reason}")]
    InvalidBucketName {
        /// Which rule the name broke.
        reason: &'static str,
    },
    /// The object key is empty.
    #[error("an object key may not be empty")]
    EmptyObjectKey,
    /// The object key is longer than 1,024 bytes of UTF-8; the S3 answer is `KeyTooLongError`.
    #[error("object key is {len} bytes, longer than the 1024 allowed")]
    ObjectKeyTooLong {
        /// The key's length in bytes.
        len: usize,
    },
}

impl Error {
    /// The taxonomy category of this failure.
    #[must_use]
    pub const fn category(&self) -> ErrorCategory {
        match self {
            Self::InvalidBucketName { .. }
            | Self::EmptyObjectKey
            | Self::ObjectKeyTooLong { .. } => ErrorCategory::Validation,
        }
    }
}

/// The result of an operation in this crate.
pub type Result<T> = std::result::Result<T, Error>;
