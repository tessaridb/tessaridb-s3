//! Limits of the access surface: bucket policies.

/// The largest bucket policy document accepted, in bytes (S3: 20 KB).
pub const BUCKET_POLICY_MAX_BYTES: usize = 20 * 1024;
