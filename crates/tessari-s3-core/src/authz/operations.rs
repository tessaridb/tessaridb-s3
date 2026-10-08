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
        Operation::HeadBucket
        | Operation::GetBucketLocation
        | Operation::ListObjects
        | Operation::ListObjectsV2
        | Operation::ListMultipartUploads => on(Need::ReadBucket),
        Operation::ListParts => uploading(Need::ReadBucket),
        Operation::GetObject | Operation::HeadObject | Operation::GetObjectTagging => {
            on(Need::ReadObject)
        }
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
