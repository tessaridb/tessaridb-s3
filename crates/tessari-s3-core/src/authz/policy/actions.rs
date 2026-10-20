//! The policy action each operation is asked as, and the S3 action names a policy may grant or deny. A name outside
//! the list is refused when a policy is written: a misspelt Deny would protect nothing.

use crate::dispatch::Operation;

/// Every S3 action name a bucket policy may name, whether or not this server has the operation.
pub const S3_ACTIONS: &[&str] = &[
    "AbortMultipartUpload",
    "BypassGovernanceRetention",
    "CreateBucket",
    "DeleteBucket",
    "DeleteBucketOwnershipControls",
    "DeleteBucketPolicy",
    "DeleteBucketWebsite",
    "DeleteObject",
    "DeleteObjectTagging",
    "DeleteObjectVersion",
    "DeleteObjectVersionTagging",
    "GetAccelerateConfiguration",
    "GetAnalyticsConfiguration",
    "GetBucketAcl",
    "GetBucketCORS",
    "GetBucketLocation",
    "GetBucketLogging",
    "GetBucketNotification",
    "GetBucketObjectLockConfiguration",
    "GetBucketOwnershipControls",
    "GetBucketPolicy",
    "GetBucketPolicyStatus",
    "GetBucketPublicAccessBlock",
    "GetBucketRequestPayment",
    "GetBucketTagging",
    "GetBucketVersioning",
    "GetBucketWebsite",
    "GetEncryptionConfiguration",
    "GetIntelligentTieringConfiguration",
    "GetInventoryConfiguration",
    "GetLifecycleConfiguration",
    "GetMetricsConfiguration",
    "GetObject",
    "GetObjectAcl",
    "GetObjectAttributes",
    "GetObjectLegalHold",
    "GetObjectRetention",
    "GetObjectTagging",
    "GetObjectTorrent",
    "GetObjectVersion",
    "GetObjectVersionAcl",
    "GetObjectVersionAttributes",
    "GetObjectVersionForReplication",
    "GetObjectVersionTagging",
    "GetObjectVersionTorrent",
    "GetReplicationConfiguration",
    "ListAllMyBuckets",
    "ListBucket",
    "ListBucketMultipartUploads",
    "ListBucketVersions",
    "ListMultipartUploadParts",
    "ObjectOwnerOverrideToBucketOwner",
    "PutAccelerateConfiguration",
    "PutAnalyticsConfiguration",
    "PutBucketAcl",
    "PutBucketCORS",
    "PutBucketLogging",
    "PutBucketNotification",
    "PutBucketObjectLockConfiguration",
    "PutBucketOwnershipControls",
    "PutBucketPolicy",
    "PutBucketPublicAccessBlock",
    "PutBucketRequestPayment",
    "PutBucketTagging",
    "PutBucketVersioning",
    "PutBucketWebsite",
    "PutEncryptionConfiguration",
    "PutIntelligentTieringConfiguration",
    "PutInventoryConfiguration",
    "PutLifecycleConfiguration",
    "PutMetricsConfiguration",
    "PutObject",
    "PutObjectAcl",
    "PutObjectLegalHold",
    "PutObjectRetention",
    "PutObjectTagging",
    "PutObjectVersionAcl",
    "PutObjectVersionTagging",
    "PutReplicationConfiguration",
    "ReplicateDelete",
    "ReplicateObject",
    "ReplicateTags",
    "RestoreObject",
];

/// What a policy action applies to: the bucket's own ARN, or the object's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// `arn:aws:s3:::<bucket>`.
    Bucket,
    /// `arn:aws:s3:::<bucket>/<key>`.
    Object,
}

/// The action `operation` is asked as, and what it applies to; `naming_version` selects the version-specific action
/// where S3 has one. `None` for an operation no bucket policy governs (listing every bucket, creating one).
#[must_use]
pub fn policy_action(operation: Operation, naming_version: bool) -> Option<(&'static str, Scope)> {
    use Operation as O;
    use Scope::{Bucket, Object};
    let versioned =
        |plain: &'static str, version: &'static str| if naming_version { version } else { plain };
    Some(match operation {
        O::DeleteBucket => ("s3:DeleteBucket", Bucket),
        O::HeadBucket | O::ListObjects | O::ListObjectsV2 => ("s3:ListBucket", Bucket),
        O::ListObjectVersions => ("s3:ListBucketVersions", Bucket),
        O::ListMultipartUploads => ("s3:ListBucketMultipartUploads", Bucket),
        O::GetBucketLocation => ("s3:GetBucketLocation", Bucket),
        O::GetBucketVersioning => ("s3:GetBucketVersioning", Bucket),
        O::PutBucketVersioning => ("s3:PutBucketVersioning", Bucket),
        O::GetBucketEncryption => ("s3:GetEncryptionConfiguration", Bucket),
        O::PutBucketEncryption | O::DeleteBucketEncryption => {
            ("s3:PutEncryptionConfiguration", Bucket)
        }
        O::GetBucketLifecycleConfiguration => ("s3:GetLifecycleConfiguration", Bucket),
        O::PutBucketLifecycleConfiguration | O::DeleteBucketLifecycle => {
            ("s3:PutLifecycleConfiguration", Bucket)
        }
        O::GetBucketNotificationConfiguration => ("s3:GetBucketNotification", Bucket),
        O::PutBucketNotificationConfiguration => ("s3:PutBucketNotification", Bucket),
        O::GetBucketReplication => ("s3:GetReplicationConfiguration", Bucket),
        O::PutBucketReplication | O::DeleteBucketReplication => {
            ("s3:PutReplicationConfiguration", Bucket)
        }
        O::GetObjectLockConfiguration => ("s3:GetBucketObjectLockConfiguration", Bucket),
        O::PutObjectLockConfiguration => ("s3:PutBucketObjectLockConfiguration", Bucket),
        O::GetBucketTagging => ("s3:GetBucketTagging", Bucket),
        O::PutBucketTagging | O::DeleteBucketTagging => ("s3:PutBucketTagging", Bucket),
        O::GetBucketPolicy => ("s3:GetBucketPolicy", Bucket),
        O::PutBucketPolicy => ("s3:PutBucketPolicy", Bucket),
        O::DeleteBucketPolicy => ("s3:DeleteBucketPolicy", Bucket),
        O::GetBucketPolicyStatus => ("s3:GetBucketPolicyStatus", Bucket),
        O::GetPublicAccessBlock => ("s3:GetBucketPublicAccessBlock", Bucket),
        O::PutPublicAccessBlock | O::DeletePublicAccessBlock => {
            ("s3:PutBucketPublicAccessBlock", Bucket)
        }
        O::GetBucketCors => ("s3:GetBucketCORS", Bucket),
        O::PutBucketCors | O::DeleteBucketCors => ("s3:PutBucketCORS", Bucket),
        O::GetObject | O::HeadObject => (versioned("s3:GetObject", "s3:GetObjectVersion"), Object),
        O::GetObjectTagging => (
            versioned("s3:GetObjectTagging", "s3:GetObjectVersionTagging"),
            Object,
        ),
        O::PutObjectTagging => (
            versioned("s3:PutObjectTagging", "s3:PutObjectVersionTagging"),
            Object,
        ),
        O::DeleteObjectTagging => (
            versioned("s3:DeleteObjectTagging", "s3:DeleteObjectVersionTagging"),
            Object,
        ),
        O::GetBucketAcl => ("s3:GetBucketAcl", Bucket),
        O::PutBucketAcl => ("s3:PutBucketAcl", Bucket),
        O::GetBucketOwnershipControls => ("s3:GetBucketOwnershipControls", Bucket),
        O::PutBucketOwnershipControls => ("s3:PutBucketOwnershipControls", Bucket),
        O::GetObjectAcl => (
            versioned("s3:GetObjectAcl", "s3:GetObjectVersionAcl"),
            Object,
        ),
        O::PutObjectAcl => (
            versioned("s3:PutObjectAcl", "s3:PutObjectVersionAcl"),
            Object,
        ),
        O::GetObjectRetention => ("s3:GetObjectRetention", Object),
        O::PutObjectRetention => ("s3:PutObjectRetention", Object),
        O::GetObjectLegalHold => ("s3:GetObjectLegalHold", Object),
        O::PutObjectLegalHold => ("s3:PutObjectLegalHold", Object),
        O::DeleteObject => (
            versioned("s3:DeleteObject", "s3:DeleteObjectVersion"),
            Object,
        ),
        O::PutObject
        | O::CopyObject
        | O::CreateMultipartUpload
        | O::UploadPart
        | O::UploadPartCopy
        | O::CompleteMultipartUpload => ("s3:PutObject", Object),
        O::AbortMultipartUpload => ("s3:AbortMultipartUpload", Object),
        O::ListParts => ("s3:ListMultipartUploadParts", Object),
        // Each key of the batch is asked as its own delete; the batch itself needs no action of its own.
        O::DeleteObjects => ("s3:DeleteObject", Object),
        _ => return None,
    })
}
