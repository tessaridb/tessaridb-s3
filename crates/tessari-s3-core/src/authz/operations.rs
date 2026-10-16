//! What each implemented S3 operation needs from the evaluator. The table sits beside the dispatch catalog, and a test
//! fails when an implemented operation has no row, so a new handler cannot ship unauthorized.

use crate::dispatch::Operation;

/// The kind of permission an operation needs; the caller supplies the bucket or space it applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    /// List the caller's buckets.
    ListBuckets,
    /// Create a bucket in the caller's space.
    CreateBucket,
    /// Delete the named bucket.
    DeleteBucket,
    /// Change the named bucket's configuration (versioning).
    ConfigureBucket,
    /// Describe the bucket or list what it holds.
    ReadBucket,
    /// Read an object of the bucket.
    ReadObject,
    /// Write or delete an object of the bucket.
    WriteObject,
}

/// An operation's permission needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Required {
    /// The permission on the target bucket (or the space, for `CreateBucket`).
    pub primary: Need,
    /// Whether the copy source must also be readable, checked against the source's own bucket.
    pub source_read: bool,
    /// Whether it is a step of uploading the one key it addresses — all a one-key credential may do.
    pub upload: bool,
}

const fn on(primary: Need) -> Option<Required> {
    Some(Required {
        primary,
        source_read: false,
        upload: false,
    })
}

const fn uploading(primary: Need) -> Option<Required> {
    Some(Required {
        primary,
        source_read: false,
        upload: true,
    })
}

const fn copying() -> Option<Required> {
    Some(Required {
        primary: Need::WriteObject,
        source_read: true,
        upload: false,
    })
}

/// What `operation` needs, or `None` for an operation with no handler (it is refused with 501 before this matters).
#[must_use]
pub const fn required(operation: Operation) -> Option<Required> {
    match operation {
        Operation::ListBuckets => on(Need::ListBuckets),
        Operation::CreateBucket => on(Need::CreateBucket),
        Operation::DeleteBucket => on(Need::DeleteBucket),
        Operation::PutBucketVersioning
        | Operation::PutBucketLifecycleConfiguration
        | Operation::PutBucketNotificationConfiguration
        | Operation::PutBucketReplication
        | Operation::DeleteBucketReplication
        | Operation::DeleteBucketLifecycle
        | Operation::PutObjectLockConfiguration
        // Releasing a legal hold must not be a plain write: a hold exists to outlast who may write.
        | Operation::PutObjectLegalHold => on(Need::ConfigureBucket),
        Operation::HeadBucket
        | Operation::GetBucketLocation
        | Operation::GetBucketVersioning
        | Operation::GetBucketLifecycleConfiguration
        | Operation::GetBucketNotificationConfiguration
        | Operation::GetBucketReplication
        | Operation::GetObjectLockConfiguration
        | Operation::ListObjects
        | Operation::ListObjectsV2
        | Operation::ListObjectVersions
        | Operation::ListMultipartUploads => on(Need::ReadBucket),
        Operation::ListParts => uploading(Need::ReadBucket),
        Operation::GetObject
        | Operation::HeadObject
        | Operation::GetObjectTagging
        | Operation::GetObjectRetention
        | Operation::GetObjectLegalHold => on(Need::ReadObject),
        // Extending a retention is a write; shortening a GOVERNANCE one also needs who administers the bucket.
        Operation::PutObjectRetention => on(Need::WriteObject),
        Operation::DeleteObject | Operation::DeleteObjects => on(Need::WriteObject),
        Operation::PutObject
        | Operation::CreateMultipartUpload
        | Operation::UploadPart
        | Operation::CompleteMultipartUpload
        | Operation::AbortMultipartUpload => uploading(Need::WriteObject),
        Operation::CopyObject | Operation::UploadPartCopy => copying(),
        _ => None,
    }
}

/// What `operation` needs when the request names a version (`versionId`, or a replica's source version): deleting a
/// version is permanent — the one loss versioning exists to prevent — and a replica writes a version id of its
/// sender's choosing, so both need who administers the bucket; anything else needs what it always does.
#[must_use]
pub const fn required_naming_version(operation: Operation) -> Option<Required> {
    match operation {
        Operation::DeleteObject => on(Need::ConfigureBucket),
        // A replica write keeps the version id its source gave it, so the writer chooses the bucket's history.
        Operation::PutObject => on(Need::ConfigureBucket),
        other => required(other),
    }
}
