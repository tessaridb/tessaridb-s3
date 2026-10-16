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
    /// The cluster has no layout yet: fewer nodes have registered than the erasure code has shards.
    #[error("the cluster has no layout yet; fewer nodes than shards have registered")]
    NoLayout,
    /// Fewer shards than the read quorum answered with a verified block; nothing of the stripe is returned.
    #[error("{have} shards readable, the read quorum is {needed}")]
    Unreadable {
        /// Shards that answered with a verified block.
        have: u8,
        /// The read quorum (k).
        needed: u8,
    },
    /// Fewer shards than the write quorum were made durable; the write is not acknowledged.
    #[error("{durable} shards durable, the write quorum is {needed}")]
    Quorum {
        /// Shards that reached their node's disk (or could still, when the write stopped early).
        durable: u8,
        /// The write quorum.
        needed: u8,
    },
    /// Another node answered a shard request with a refusal.
    #[error("a peer refused the shard request with status {status}")]
    PeerRefused {
        /// The HTTP status it answered.
        status: u16,
    },
    /// The bucket lives in another region: its object metadata is in that region's database, so this node neither
    /// reads nor writes it, and the request belongs at that region's nodes.
    #[error("the bucket lives in region {region}")]
    OtherRegion {
        /// The bucket's region.
        region: String,
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
    /// An access key was to be issued or resolved and no root key (`TESSARIDB_S3_IAM_KEY`) is configured.
    #[error("no IAM root key is configured (TESSARIDB_S3_IAM_KEY)")]
    NoIamKey,
    /// A sealed value did not open — an access-key secret, a wrapped data key or a block of object data: another key,
    /// another binding, a changed byte or an unknown algorithm, deliberately not told apart.
    #[error("a sealed value could not be opened")]
    Unsealable,
    /// Another node holds the claim on this data id (it is healing or reclaiming it); try on a later pass.
    #[error("another node is working on this data id")]
    Held,
    /// This node holds no encryption keys: an operator has not unsealed them since it started.
    #[error("the encryption keys are sealed on this node")]
    KeysSealed,
    /// A key was to be created under an id the keys vault already holds; keys are never replaced.
    #[error("the keys vault already holds {id}")]
    KeyExists {
        /// The record id.
        id: String,
    },
    /// An object was to be sealed under a named key this node does not hold.
    #[error("no encryption key is named {name}")]
    UnknownKey {
        /// The name asked for.
        name: String,
    },
    /// A keys passphrase that cannot be used; the reason never quotes it.
    #[error("the keys passphrase is not usable: {reason}")]
    WeakPassphrase {
        /// What is wrong with it.
        reason: &'static str,
    },
    /// The keys vault did not open with the passphrase given.
    #[error("the keys passphrase is wrong")]
    WrongPassphrase,
    /// A sealed object's body was not the length declared before its first byte; nothing of it is kept.
    #[error("a sealed body declared {declared} bytes and carried {received}")]
    SealedLength {
        /// Bytes declared.
        declared: u64,
        /// Bytes carried (when too many, those offered up to the refusal).
        received: u64,
    },
    /// Too many wrong passphrases in a row: the metadata store makes the next attempt wait, right or not.
    #[error("too many wrong keys passphrases; wait before trying again")]
    PassphraseThrottled,
}

impl Error {
    /// The taxonomy category.
    #[must_use]
    pub fn category(&self) -> ErrorCategory {
        match self {
            Self::Meta(meta) => meta.category(),
            Self::Peer(peer) => peer.category(),
            Self::Quorum { .. } | Self::Unreadable { .. } | Self::NoLayout => {
                ErrorCategory::Unavailable
            }
            Self::PeerRefused { status: 404 } => ErrorCategory::NotFound,
            Self::PeerRefused { status: 409 } => ErrorCategory::Conflict,
            Self::PeerRefused { status: 503 } => ErrorCategory::Unavailable,
            Self::PeerRefused { .. } => ErrorCategory::Internal,
            Self::Malformed { .. }
            | Self::DataIo { .. }
            | Self::Corrupt { .. }
            | Self::Randomness
            | Self::Unsealable
            | Self::NoIamKey
            | Self::LayoutMismatch { .. } => ErrorCategory::Internal,
            Self::InvalidShard { .. }
            | Self::ShardLength { .. }
            | Self::SealedLength { .. }
            | Self::OtherRegion { .. } => ErrorCategory::Validation,
            Self::Contended | Self::Held | Self::KeyExists { .. } => ErrorCategory::Conflict,
            Self::KeysSealed | Self::PassphraseThrottled => ErrorCategory::Unavailable,
            Self::WeakPassphrase { .. } | Self::UnknownKey { .. } => ErrorCategory::Validation,
            Self::WrongPassphrase => ErrorCategory::Unauthorized,
        }
    }
}

/// The result of a storage operation.
pub type Result<T> = std::result::Result<T, Error>;
