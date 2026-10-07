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
    /// A data file operation failed at the operating system. After a failed sync the write is over: the pages the
    /// sync covered may be gone even though a second sync would succeed.
    #[error("data file {op} failed: {kind}")]
    DataIo {
        /// Which operation (`create`, `write`, `sync`, `rename`, `read` …).
        op: &'static str,
        /// The operating system's error kind; the message is logged where it happened.
        kind: std::io::ErrorKind,
    },
    /// A data file does not hold what its record says it should — the media or the file was damaged.
    #[error("data file {id} is damaged: {reason}")]
    Corrupt {
        /// The data id, hex.
        id: String,
        /// What failed verification.
        reason: &'static str,
    },
    /// An object needs a data file and no data directory is configured.
    #[error("no data directory is configured (TESSARIDB_S3_DATA_DIR)")]
    NoDataDirectory,
    /// The operating system's random source failed, so no unguessable id could be drawn.
    #[error("the random source failed")]
    Randomness,
}

impl Error {
    /// The taxonomy category.
    #[must_use]
    pub fn category(&self) -> ErrorCategory {
        match self {
            Self::Meta(meta) => meta.category(),
            Self::Malformed { .. }
            | Self::DataIo { .. }
            | Self::Corrupt { .. }
            | Self::NoDataDirectory
            | Self::Randomness => ErrorCategory::Internal,
        }
    }
}

/// The result of a storage operation.
pub type Result<T> = std::result::Result<T, Error>;
