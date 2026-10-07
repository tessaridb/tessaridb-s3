//! What object persistence must answer.

use std::future::Future;

use tessari_s3_types::{BucketName, ObjectKey};

use super::model::{NewObject, ObjectSummary, StoredObject};
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
    /// Writes `object` under `guard` as ONE statement: UPSERT, CREATE, or UPDATE … WHERE. When the object's bytes
    /// are in a data file, the same transaction removes that file's pending `gc` entry.
    fn write(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        incarnation: [u8; 16],
        object: &NewObject,
        guard: &Guard,
    ) -> impl Future<Output = Result<Wrote>> + Send;
    /// Queues data `id` for reclamation; done before its file is created.
    fn queue(&self, id: [u8; 16]) -> impl Future<Output = Result<()>> + Send;
    /// Takes data `id` off the reclamation queue, after its file was removed.
    fn unqueue(&self, id: [u8; 16]) -> impl Future<Output = Result<()>> + Send;
    /// Queue entries older than `grace_secs` by the store's clock, at most `limit`, each with whether a reclaimer
    /// already marked it.
    fn due(
        &self,
        grace_secs: u64,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<([u8; 16], bool)>>> + Send;
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
    /// Removes the record at `bucket/key`, if any.
    fn remove(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
    ) -> impl Future<Output = Result<()>> + Send;
}
