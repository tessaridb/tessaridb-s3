//! What object persistence must answer.

use std::future::Future;

use tessari_s3_types::{BucketName, ObjectKey};

use tessari_s3_core::versioning::Versioning;

use super::model::{NewObject, ObjectSummary, Part, StoredObject};
use crate::Result;

/// What one write statement requires of the record already at the key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Guard {
    /// Nothing: overwrite.
    None,
    /// No record at all.
    Absent,
    /// A record of the current incarnation with this ETag.
    Etag(String),
    /// A record left from this earlier incarnation of the bucket.
    Incarnation([u8; 16]),
}

/// The outcome of one conditional write statement.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Wrote {
    /// Committed; the record as it now stands.
    Record(tessari_s3_infrastructure::tessaridb::Value),
    /// The statement's condition did not hold, or the record it named was absent.
    Refused,
}

/// What a version write needs to know about a key, read at one snapshot (ADR-0010).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Head {
    /// The bucket's versioning state; `None` while it was never versioned.
    pub(crate) state: Option<Versioning>,
    /// The sequence of the key's newest version.
    pub(crate) newest: Option<u64>,
    /// The sequence of the key's null version.
    pub(crate) null: Option<u64>,
    /// Whether the bucket has a notification configuration or media rules: a change then leaves its event
    /// (`object_events`).
    pub(crate) notifies: bool,
    /// Whether the key has a current object record.
    pub(crate) current: bool,
    /// Whether the bucket asked for its writes to be acknowledged by its region's majority alone.
    pub(crate) local: bool,
    /// The bucket's replication configuration; `None` while it has none.
    pub(crate) replication: Option<tessari_s3_core::replication::Configuration>,
}

/// The version a write or a delete adds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NewVersion {
    /// Its sequence.
    pub(crate) seq: u64,
    /// Its id: `null`, or the sequence and a nonce.
    pub(crate) version_id: String,
}

/// The version records one write or delete changes, in the transaction that writes or removes the key's `objects`
/// record: the null version is dropped when the new one replaces it, and the new version is created after the newest.
/// Two writers that read the same head collide on that `objects` record and on the new version's id, so one commit is
/// refused and that writer reads the head again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VersionPlan {
    /// The null version the new one replaces.
    pub(crate) drop_null: Option<u64>,
    /// The version added; `None` for a delete in a never-versioned bucket, which leaves the key nothing.
    pub(crate) create: Option<NewVersion>,
    /// The version id the current object answers with; `None` while the bucket was never versioned.
    pub(crate) label: Option<String>,
    /// Whether the bucket notifies: stamped on the record the change writes or removes, which is what lets the store's
    /// own events decide whether the change leaves an `object_events` record.
    pub(crate) notify: bool,
    /// Whether the key has a current object record to stamp before a removal.
    pub(crate) current: bool,
    /// What a removal is done for, which names its event.
    pub(crate) removal: Removal,
    /// Whether the change is acknowledged by the bucket's region's majority (`ACKNOWLEDGE LOCAL MAJORITY`) rather
    /// than the namespace's.
    pub(crate) local: bool,
    /// The bucket the version this change adds is replicated into: stamped `PENDING` on the version, which is what
    /// lets the store's own event queue its replication task in this commit.
    pub(crate) replicate: Option<String>,
    /// Whether the version this change adds is a replica another bucket sent: it keeps the id it was sent with, is
    /// stamped `REPLICA`, and is never replicated again.
    pub(crate) replica: bool,
}

impl VersionPlan {
    /// The replication status the version this change adds is stamped with, if any.
    pub(crate) const fn replication_status(&self) -> Option<&'static str> {
        if self.replica {
            Some("REPLICA")
        } else if self.replicate.is_some() {
            Some("PENDING")
        } else {
            None
        }
    }
}

/// What removes an object or a version: a request, or the bucket's lifecycle rules (S3 ref 11 M5 event names).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Removal {
    /// A DeleteObject or DeleteObjects request.
    Request,
    /// A lifecycle expiration.
    Lifecycle,
}

impl Removal {
    /// The event family the removal is named under.
    pub(crate) const fn family(self) -> &'static str {
        match self {
            Self::Request => "ObjectRemoved",
            Self::Lifecycle => "LifecycleExpiration",
        }
    }
}

/// A bucket's incarnation and the record at one key, read at one snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Snapshot {
    /// The bucket's current incarnation; `None` when the bucket does not exist.
    pub(crate) bucket: Option<[u8; 16]>,
    /// The record at the key with the incarnation it was written under.
    pub(crate) object: Option<([u8; 16], StoredObject)>,
}

/// One object record as a listing reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ListRow {
    /// The bucket the record was written in.
    pub(crate) bucket: String,
    /// The object key.
    pub(crate) key: String,
    /// The bucket incarnation it was written under.
    pub(crate) incarnation: [u8; 16],
    /// What a listing shows.
    pub(crate) summary: ObjectSummary,
}

/// Object records in record-id order from an anchor, with the bucket's incarnation, read at one snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Batch {
    /// The bucket's current incarnation; `None` when the bucket does not exist.
    pub(crate) bucket: Option<[u8; 16]>,
    /// The records, the anchor's own first when it was asked for and exists.
    pub(crate) rows: Vec<ListRow>,
    /// Whether the read stopped at its limit, so records may follow.
    pub(crate) full: bool,
}

/// Object queries.
pub(crate) trait ObjectRepository: Send + Sync {
    /// The bucket's incarnation, or `None` when the bucket does not exist.
    fn incarnation(
        &self,
        bucket: &BucketName,
    ) -> impl Future<Output = Result<Option<[u8; 16]>>> + Send;
    /// The record at `bucket/key` with its incarnation, read with the bucket's in one snapshot.
    fn read(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
    ) -> impl Future<Output = Result<Snapshot>> + Send;
    /// The bucket's versioning state and the key's newest and null versions, read at one snapshot.
    fn head(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
    ) -> impl Future<Output = Result<Head>> + Send;
    /// Writes `object` under `guard` as ONE statement: UPSERT, CREATE, or UPDATE … WHERE — and, in the same
    /// transaction, the version records `versions` names. When the object's bytes are in a data file, the same
    /// transaction removes that file's pending `reclaims` entry.
    fn write(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        incarnation: [u8; 16],
        object: &NewObject,
        guard: &Guard,
        versions: &VersionPlan,
    ) -> impl Future<Output = Result<Wrote>> + Send;
    /// Queues data `id` for reclamation; done before its file is created.
    fn queue(&self, id: [u8; 16]) -> impl Future<Output = Result<()>> + Send;
    /// Takes data `id` off the reclamation queue, after its file was removed.
    fn unqueue(&self, id: [u8; 16]) -> impl Future<Output = Result<()>> + Send;
    /// Takes and holds the next queue entry past its grace, with whether a reclaimer already marked it; `None` when
    /// none is due. A hold this node does not finish lapses and the entry is handed out again.
    fn claim_next(&self) -> impl Future<Output = Result<Option<([u8; 16], bool)>>> + Send;
    /// Hands held data `id` back to the queue, not to be handed out again for a while.
    fn defer(&self, id: [u8; 16]) -> impl Future<Output = Result<()>> + Send;
    /// Whether any object or multipart part record references data `id`.
    fn referenced(&self, id: [u8; 16]) -> impl Future<Output = Result<bool>> + Send;
    /// Marks data `id` as being reclaimed; `false` when its entry is gone or already marked.
    fn mark(&self, id: [u8; 16]) -> impl Future<Output = Result<bool>> + Send;
    /// Whether data `id` is still queued and not being reclaimed — a data commit can still take it.
    fn claimable(&self, id: [u8; 16]) -> impl Future<Output = Result<bool>> + Send;
    /// At most `limit` object records after `bucket/after` in record-id order — record ids order by UTF-8 bytes, so
    /// a bucket's keys come in S3's listing order — preceded by the record at `bucket/at` itself when `at` is given.
    /// Rows past the bucket are included; the caller stops at them.
    fn page(
        &self,
        bucket: &BucketName,
        at: Option<&str>,
        after: &str,
        limit: usize,
    ) -> impl Future<Output = Result<Batch>> + Send;
    /// The parts of the multipart object at `bucket/key` in number order, read in one snapshot with the object — or
    /// `None` when the object there is no longer the one completed from `upload`.
    fn parts_of(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        upload: [u8; 16],
    ) -> impl Future<Output = Result<Option<Vec<Part>>>> + Send;
    /// Removes the record at `bucket/key` only while it is of `incarnation`, its ETag is `etag` and — when given — it
    /// was written at `modified`, with the version records `versions` names, as ONE transaction; `false` when that did
    /// not hold or there was no record.
    fn remove_if(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        incarnation: [u8; 16],
        etag: &str,
        modified: Option<tessari_s3_types::Timestamp>,
        versions: &VersionPlan,
    ) -> impl Future<Output = Result<bool>> + Send;
    /// Removes the record at `bucket/key`, if any, with the version records `versions` names, as ONE transaction;
    /// `false` when the key's versions moved on since they were read.
    fn remove(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        incarnation: [u8; 16],
        versions: &VersionPlan,
    ) -> impl Future<Output = Result<bool>> + Send;
}
