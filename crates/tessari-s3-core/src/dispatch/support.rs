//! Which operations this server implements. Every other catalog operation is answered `NotImplemented` (501).

use super::catalog::Operation;

/// Operations with a handler; every other catalog operation is refused with 501.
const IMPLEMENTED: &[Operation] = &[
    Operation::AbortMultipartUpload,
    Operation::CompleteMultipartUpload,
    Operation::CopyObject,
    Operation::CreateBucket,
    Operation::CreateMultipartUpload,
    Operation::DeleteBucket,
    Operation::DeleteObject,
    Operation::DeleteObjects,
    Operation::GetBucketLocation,
    Operation::GetObject,
    Operation::GetObjectTagging,
    Operation::HeadBucket,
    Operation::HeadObject,
    Operation::ListBuckets,
    Operation::ListMultipartUploads,
    Operation::ListObjects,
    Operation::ListObjectsV2,
    Operation::ListParts,
    Operation::PutObject,
    Operation::UploadPart,
    Operation::UploadPartCopy,
];

/// Whether `operation` has a handler; `false` means the request is answered `NotImplemented`.
#[must_use]
pub fn is_implemented(operation: Operation) -> bool {
    IMPLEMENTED.contains(&operation)
}
