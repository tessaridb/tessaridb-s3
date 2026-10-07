//! Which operations this server implements. Every other catalog operation is answered `NotImplemented` (501).

use super::catalog::Operation;

/// Operations with a handler; every other catalog operation is refused with 501.
const IMPLEMENTED: &[Operation] = &[
    Operation::CreateBucket,
    Operation::DeleteBucket,
    Operation::DeleteObject,
    Operation::GetBucketLocation,
    Operation::GetObject,
    Operation::HeadBucket,
    Operation::HeadObject,
    Operation::ListBuckets,
    Operation::ListObjects,
    Operation::ListObjectsV2,
    Operation::PutObject,
];

/// Whether `operation` has a handler; `false` means the request is answered `NotImplemented`.
#[must_use]
pub fn is_implemented(operation: Operation) -> bool {
    IMPLEMENTED.contains(&operation)
}
