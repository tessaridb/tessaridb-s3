//! What object persistence must answer.

use std::future::Future;

use tessari_s3_types::{BucketName, ObjectKey};

use super::model::{NewObject, StoredObject};
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
    /// Writes `object` under `guard` as ONE statement: UPSERT, CREATE, or UPDATE … WHERE.
    fn write(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        incarnation: [u8; 16],
        object: &NewObject,
        guard: &Guard,
    ) -> impl Future<Output = Result<Wrote>> + Send;
    /// Removes the record at `bucket/key`, if any.
    fn remove(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
    ) -> impl Future<Output = Result<()>> + Send;
}
