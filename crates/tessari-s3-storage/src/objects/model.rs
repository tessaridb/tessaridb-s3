//! An object as the rest of the server sees it, and what a write asks for.

use std::collections::BTreeMap;

use tessari_s3_types::Timestamp;

/// A new object's metadata and inline bytes, ready to commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewObject {
    /// Size in bytes.
    pub size: u64,
    /// The quoted ETag.
    pub etag: String,
    /// Stored system headers, lowercase names (`content-type`, `cache-control` …).
    pub headers: BTreeMap<String, String>,
    /// User metadata, lowercase names without the `x-amz-meta-` prefix.
    pub metadata: BTreeMap<String, String>,
    /// Checksums by algorithm name (`CRC64NVME` …), base64.
    pub checksums: BTreeMap<String, String>,
    /// The object's bytes, stored in the metadata record.
    pub inline: Vec<u8>,
}

/// A stored object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredObject {
    /// Size in bytes.
    pub size: u64,
    /// The quoted ETag.
    pub etag: String,
    /// When it was written.
    pub modified: Timestamp,
    /// Stored system headers.
    pub headers: BTreeMap<String, String>,
    /// User metadata.
    pub metadata: BTreeMap<String, String>,
    /// Checksums by algorithm name.
    pub checksums: BTreeMap<String, String>,
    /// The bytes.
    pub inline: Vec<u8>,
}

/// The condition a write is made under; each is one atomic step on the key's record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteCondition {
    /// Overwrite whatever is there.
    None,
    /// `If-None-Match: *` — only if no object exists.
    IfNoneMatch,
    /// `If-Match: <etag>` — only if the current object has this quoted ETag.
    IfMatch(String),
}

/// What a write ended in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Written {
    /// Committed at this instant.
    Committed(Timestamp),
    /// The bucket does not exist.
    NoSuchBucket,
    /// The condition did not hold: 412.
    PreconditionFailed,
    /// `If-Match` named an object that does not exist: 404.
    NoSuchKey,
}

/// What a delete ended in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Removed {
    /// The key holds no object now (whether or not it did before).
    Done,
    /// The bucket does not exist.
    NoSuchBucket,
}
