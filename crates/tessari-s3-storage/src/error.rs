//! Failures of `tessari-s3-storage`.

use tessari_s3_infrastructure::peer::PeerError;
use tessari_s3_infrastructure::tessaridb::MetaError;
use tessari_s3_types::ErrorCategory;

/// A storage operation that did not complete.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The metadata store refused or could not be reached.
    #[error(transparent)]
    Meta(#[from] MetaError),
    /// Another node of the cluster could not be reached, or stopped answering.
    #[error(transparent)]
    Peer(#[from] PeerError),
    /// Another node answered a shard request with a refusal.
    #[error("a peer refused the shard request with status {status}")]
    PeerRefused {
        /// The HTTP status it answered.
        status: u16,
    },
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
    /// A shard request the shard store does not take: a block size of zero or past a data block, or an index past
    /// the widest code.
    #[error("the shard request is not valid: {reason}")]
    InvalidShard {
        /// What is wrong with it.
        reason: &'static str,
    },
    /// A shard's body was not the length its request declared; nothing of it was kept.
    #[error("a shard declared {declared} bytes and carried {received}")]
    ShardLength {
        /// Bytes the request declared.
        declared: u64,
        /// Bytes it carried (when it carried too many, those received up to the refusal).
        received: u64,
    },
    /// The operating system's random source failed, so no unguessable id could be drawn.
    #[error("the random source failed")]
    Randomness,
    /// The stored layout was written under another erasure code than this node is configured with; the node does
    /// not reinterpret it.
    #[error(
        "the cluster's layout uses {stored_data}+{stored_parity}, this node is configured for {data}+{parity}"
    )]
    LayoutMismatch {
        /// k of the stored layout.
        stored_data: i64,
        /// m of the stored layout.
        stored_parity: i64,
        /// k configured here.
        data: u8,
        /// m configured here.
        parity: u8,
    },
    /// A key kept changing under a read that needs two snapshots (a multipart object's record, then its parts);
    /// retryable.
    #[error("the key changed under every read attempt")]
    Contended,
}

impl Error {
    /// The taxonomy category.
    #[must_use]
    pub fn category(&self) -> ErrorCategory {
        match self {
            Self::Meta(meta) => meta.category(),
            Self::Peer(peer) => peer.category(),
            Self::PeerRefused { status: 404 } => ErrorCategory::NotFound,
            Self::PeerRefused { status: 409 } => ErrorCategory::Conflict,
            Self::PeerRefused { status: 503 } => ErrorCategory::Unavailable,
            Self::PeerRefused { .. } => ErrorCategory::Internal,
            Self::Malformed { .. }
            | Self::DataIo { .. }
            | Self::Corrupt { .. }
            | Self::NoDataDirectory
            | Self::Randomness
            | Self::LayoutMismatch { .. } => ErrorCategory::Internal,
            Self::InvalidShard { .. } | Self::ShardLength { .. } => ErrorCategory::Validation,
            Self::Contended => ErrorCategory::Conflict,
        }
    }
}

/// The result of a storage operation.
pub type Result<T> = std::result::Result<T, Error>;
