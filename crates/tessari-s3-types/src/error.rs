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
    /// The upload id is not one this server could have issued; the S3 answer is `NoSuchUpload`.
    #[error("not an upload id")]
    InvalidUploadId,
    /// The part number is not an integer from 1 to 10,000; the S3 answer is `InvalidArgument`.
    #[error("a part number is an integer from 1 to 10000")]
    InvalidPartNumber,
    /// A cluster node id outside its alphabet (configuration, never a client request).
    #[error("a node id is 1 to 63 lowercase letters, digits and inner hyphens")]
    InvalidNodeId,
}

impl Error {
    /// The taxonomy category of this failure.
    #[must_use]
    pub const fn category(&self) -> ErrorCategory {
        match self {
            Self::InvalidBucketName { .. }
            | Self::EmptyObjectKey
            | Self::ObjectKeyTooLong { .. }
            | Self::InvalidUploadId
            | Self::InvalidPartNumber
            | Self::InvalidNodeId => ErrorCategory::Validation,
        }
    }
}

/// The result of an operation in this crate.
pub type Result<T> = std::result::Result<T, Error>;
