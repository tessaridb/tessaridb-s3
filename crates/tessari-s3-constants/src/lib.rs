//! Every number with a meaning in TessariDB S3, typed and documented with its unit and its reason.

/// Shortest bucket name, in bytes. AWS general purpose bucket naming rules.
pub const BUCKET_NAME_MIN_LEN: usize = 3;

/// Longest bucket name, in bytes. AWS general purpose bucket naming rules; a longer name is not a DNS label.
pub const BUCKET_NAME_MAX_LEN: usize = 63;

/// Bucket-name prefixes AWS reserves for its own namespaces; a bucket may not start with one.
pub const BUCKET_NAME_RESERVED_PREFIXES: [&str; 3] = ["xn--", "sthree-", "amzn-s3-demo-"];

/// Bucket-name suffixes AWS reserves for aliases, directory, table and account-regional buckets.
pub const BUCKET_NAME_RESERVED_SUFFIXES: [&str; 6] = [
    "-s3alias",
    "--ol-s3",
    ".mrap",
    "--x-s3",
    "--table-s3",
    "-an",
];

/// Longest object key, in bytes of UTF-8. A longer key is `KeyTooLongError`.
pub const OBJECT_KEY_MAX_LEN: usize = 1024;

/// Largest difference between the request's signing time and the server clock, in seconds (15 minutes). A request
/// outside it is `RequestTimeTooSkewed`; the window is SigV4's replay defence.
pub const SIGV4_MAX_CLOCK_SKEW_SECS: i64 = 15 * 60;

/// Shortest presigned-URL lifetime, in seconds (`X-Amz-Expires`).
pub const PRESIGN_MIN_EXPIRES_SECS: i64 = 1;

/// Longest presigned-URL lifetime, in seconds (seven days): a SigV4 signing key is valid for at most seven days.
pub const PRESIGN_MAX_EXPIRES_SECS: i64 = 7 * 24 * 60 * 60;

/// Smallest size, in bytes, of every aws-chunked data chunk except the last (8 KiB).
pub const STREAMING_MIN_CHUNK_LEN: u64 = 8 * 1024;
