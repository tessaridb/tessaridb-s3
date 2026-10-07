//! Failures of `tessari-s3-storage`.

use tessari_s3_infrastructure::tessaridb::MetaError;
use tessari_s3_types::ErrorCategory;

/// A storage operation that did not complete.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The metadata store refused or could not be reached.
    #[error(transparent)]
    Meta(#[from] MetaError),
    /// A stored record does not have the shape this server writes — a defect or a store written by something else.
    #[error("a stored {record} record is malformed: {reason}")]
    Malformed {
        /// Which kind of record.
        record: &'static str,
        /// What is wrong with it.
        reason: &'static str,
    },
}

impl Error {
    /// The taxonomy category.
    #[must_use]
    pub fn category(&self) -> ErrorCategory {
        match self {
            Self::Meta(meta) => meta.category(),
            Self::Malformed { .. } => ErrorCategory::Internal,
        }
    }
}

/// The result of a storage operation.
pub type Result<T> = std::result::Result<T, Error>;
