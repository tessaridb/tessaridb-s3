//! An object as the rest of the server sees it, and what a write asks for.

use std::collections::BTreeMap;

use tessari_s3_types::Timestamp;

/// Where an object's bytes are: always on the drives, never in the metadata record (ADR-0008).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
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
    /// Whether CopyObject made it — the change a notification names (`s3:ObjectCreated:Copy` rather than `Put`).
    pub copied: bool,
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
    /// The version id, once the bucket has been versioned (`null` for a null version); none before that.
    pub version: Option<String>,
    /// Its replication status (`PENDING`, `COMPLETED`, `FAILED` or `REPLICA`); none when it is not replicated.
    pub replication: Option<String>,
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
    /// Committed at `modified`; `version` is the id it answers with once its bucket has been versioned.
    Committed {
        /// The commit instant.
        modified: Timestamp,
        /// `null` or the new version's id; `None` while the bucket was never versioned.
        version: Option<String>,
    },
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

/// Data ids still on a layout older than the current one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MoveBacklog {
    /// Ids counted, up to the bound.
    pub listed: usize,
    /// Whether more are on older layouts than were counted.
    pub more: bool,
}

/// What one pass moving data onto the current layout did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Moved {
    /// Data ids on an older layout that were looked at.
    pub examined: usize,
    /// Ids whose shards were copied, verified and switched to the current layout.
    pub moved: usize,
    /// Ids whose shards the old layout left behind were removed, past the reclaim grace.
    pub retired: usize,
    /// Ids another node holds the claim on, left to it for this pass.
    pub held: usize,
}

/// What a delete ended in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Removed {
    /// The key holds no object now (whether or not it did before); `marker` is the id of the delete marker the delete
    /// added, once the bucket has been versioned.
    Done {
        /// The added delete marker's version id (`null` while suspended).
        marker: Option<String>,
    },
    /// The bucket does not exist.
    NoSuchBucket,
}

/// What one entry of a batch delete ended in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Deleted {
    /// A simple delete: the key holds no current object; `marker` is the id of the delete marker it added.
    Current {
        /// The added delete marker's version id, once the bucket has been versioned.
        marker: Option<String>,
    },
    /// A delete naming a version.
    Version(VersionRemoved),
}

/// What a listed version is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionKind {
    /// A version holding an object.
    Object {
        /// Size in bytes.
        size: u64,
        /// The quoted ETag.
        etag: String,
    },
    /// A delete marker.
    Marker,
    /// Only a place in the listing: the version a client's version-id-marker names. Never on a page.
    Place,
}

/// One entry of a ListObjectVersions page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedVersion {
    /// The version's record id: its place in the listing.
    pub at: Vec<u8>,
    /// The version id (`null` for the null version).
    pub version_id: String,
    /// Whether it is its key's newest version.
    pub latest: bool,
    /// When it was written.
    pub modified: Timestamp,
    /// What it is.
    pub kind: VersionKind,
}

/// What a ListObjectVersions request found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionsListed {
    /// One page.
    Page(tessari_s3_core::objects::upload_listing::UploadPage<ListedVersion>),
    /// The version-id-marker is spelled as no version this server issues.
    MalformedMarker,
    /// The bucket does not exist.
    NoSuchBucket,
}

/// What a read naming one version found (S3 ref 08 M4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionRead {
    /// The version holds an object; its parts are read when it is a multipart one.
    Object(StoredObject),
    /// The version is a delete marker, written then.
    Marker {
        /// When the marker was written.
        modified: Timestamp,
    },
    /// No version of the key has that id.
    Absent,
    /// No version this server issues is spelled so.
    Malformed,
    /// The bucket does not exist.
    NoSuchBucket,
}

/// What a delete naming one version ended in (S3 ref 08 M3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionRemoved {
    /// The version is gone; `marker` when it was a delete marker. When it was the newest, the next newest is current.
    Done {
        /// Whether the removed version was a delete marker.
        marker: bool,
    },
    /// No version of the key has that id: it is already gone.
    Absent,
    /// No version this server issues is spelled so.
    Malformed,
    /// The bucket does not exist.
    NoSuchBucket,
}

/// What a replica write ended in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Replicated {
    /// The replica is stored.
    Stored,
    /// That version was already here; nothing was written.
    Kept,
    /// Another version holds the replica's sequence here.
    Conflict,
    /// The bucket does not exist.
    NoSuchBucket,
    /// The version id is not one a replica can carry (`null`, or not an id this server issues).
    NotAVersionId,
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
    /// Of those counted, the ids handed out at least [`tessari_s3_constants::HEAL_AT_RISK_ATTEMPTS`] times.
    pub at_risk: usize,
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
    /// The `Content-Type` stored at write, if the write named one.
    pub content_type: Option<String>,
}

/// What a listing ended in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Listed {
    /// The page, in UTF-8 byte order of key.
    Page(tessari_s3_core::objects::listing::Page<ObjectSummary>),
    /// The bucket does not exist.
    NoSuchBucket,
}
