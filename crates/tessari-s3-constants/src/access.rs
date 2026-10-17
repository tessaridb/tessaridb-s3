//! Limits of the access surface: bucket and session policies.

/// The largest bucket policy document accepted, in bytes (S3: 20 KB).
pub const BUCKET_POLICY_MAX_BYTES: usize = 20 * 1024;

/// The longest session policy accepted, in characters (STS: 2 048).
pub const SESSION_POLICY_MAX_CHARS: usize = 2048;
