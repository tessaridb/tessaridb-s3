//! An object as the rest of the server sees it, and what a write asks for.

use std::collections::BTreeMap;

use tessari_s3_types::Timestamp;

/// Where an object's bytes are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    /// In the metadata record itself (objects up to the inline size).
    Inline(Vec<u8>),
    /// In the data file with this id.
    Data([u8; 16]),
    /// In the part files of a completed multipart upload.
    Parts(Multipart),
}

/// The parts a completed multipart upload left as an object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Multipart {
    /// The upload the parts belonged to; its parts records are the object's for as long as the object is.
    pub upload: [u8; 16],
    /// The parts in object order. A record read from the store leaves this empty until the object service fills it
    /// from the parts records, so outside this crate it is always complete.
    pub parts: Vec<Part>,
}

/// One part of a multipart object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Part {
    /// The part number it was uploaded as.
    pub number: u16,
    /// The data file holding it.
    pub data: [u8; 16],
    /// Size in bytes.
    pub size: u64,
}

/// A new object's metadata and where its bytes are, ready to commit.
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
    /// The object's bytes or the data file holding them.
    pub content: Content,
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
    /// The bytes or the data file holding them.
    pub content: Content,
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
    /// The object's data file was reclaimed before the commit (the upload outlived the grace period): nothing
    /// was written.
    DataReclaimed,
}

/// What one reclamation pass did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Reclaimed {
    /// Queue entries past the grace period that were looked at.
    pub examined: usize,
    /// Files removed with their entries.
    pub removed: usize,
    /// Entries an object still references: the entry is dropped, the file kept.
    pub kept: usize,
    /// Data ids whose shard bytes were on this node with no shard set left: their shards removed.
    pub orphans: usize,
}

/// What one healing pass did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Healed {
    /// Listed data ids that were looked at.
    pub examined: usize,
    /// Ids whose every shard is whole again (or whose data is gone), unlisted.
    pub healed: usize,
    /// Ids with fewer readable shards than the read quorum, left listed.
    pub at_risk: usize,
    /// Ids another node holds the claim on, left to it for this pass.
    pub held: usize,
}

/// What a delete ended in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Removed {
    /// The key holds no object now (whether or not it did before).
    Done,
    /// The bucket does not exist.
    NoSuchBucket,
}

/// What a delete conditional on the object's ETag ended in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemovedIf {
    /// The object the ETag named is gone.
    Done,
    /// The bucket does not exist.
    NoSuchBucket,
    /// The key holds no object.
    NoSuchKey,
    /// The key holds an object with another ETag: it was kept.
    PreconditionFailed,
}

/// How many data ids are listed for healing, counted up to a bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HealBacklog {
    /// Ids listed, at most the bound asked for.
    pub listed: usize,
    /// Whether more are listed than were counted.
    pub more: bool,
}

/// What a listing shows of one object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectSummary {
    /// Size in bytes.
    pub size: u64,
    /// The quoted ETag.
    pub etag: String,
    /// When it was written.
    pub modified: Timestamp,
    /// The algorithms of the checksums stored with it (`CRC64NVME` …).
    pub checksum_algorithms: Vec<String>,
}

/// What a listing ended in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Listed {
    /// The page, in UTF-8 byte order of key.
    Page(tessari_s3_core::objects::listing::Page<ObjectSummary>),
    /// The bucket does not exist.
    NoSuchBucket,
}
