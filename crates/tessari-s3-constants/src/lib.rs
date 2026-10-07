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

/// Address the S3 API listens on when `TESSARIDB_S3_LISTEN` is unset: loopback only, so an unconfigured node is not
/// reachable from the network.
pub const DEFAULT_LISTEN: &str = "127.0.0.1:9100";

/// Region the server signs for when `TESSARIDB_S3_REGION` is unset; the region every SDK assumes by default.
pub const DEFAULT_REGION: &str = "us-east-1";

/// Requests served at once before new ones are answered `SlowDown` (503), when `TESSARIDB_S3_MAX_INFLIGHT` is unset.
pub const DEFAULT_MAX_INFLIGHT: usize = 1024;

/// Seconds in-flight requests get to finish after SIGTERM, when `TESSARIDB_S3_SHUTDOWN_GRACE_SECS` is unset.
pub const DEFAULT_SHUTDOWN_GRACE_SECS: u64 = 30;

/// Shortest root secret key accepted, in bytes; AWS secret keys are 40 characters.
pub const MIN_SECRET_KEY_LEN: usize = 16;

/// Connections to the metadata store open at once, when `TESSARIDB_S3_META_MAX_CONNECTIONS` is unset.
pub const DEFAULT_META_CONNECTIONS: usize = 32;

/// Times the start-up schema is attempted when the store answers `retry` (another node is defining it at once).
pub const SCHEMA_RETRY_ATTEMPTS: u32 = 16;

/// Milliseconds of back-off per attempt between schema retries (20, 40, 60 … ms).
pub const SCHEMA_RETRY_BACKOFF_MS: u64 = 20;

/// Largest object stored inline in its metadata record, in bytes (128 KiB, ADR-0002 §1). Larger objects go to the
/// data store.
pub const INLINE_OBJECT_MAX: usize = 128 * 1024;

/// Largest sum of user metadata names and values, in UTF-8 bytes (2 KB, S3's limit; MetadataTooLarge beyond it).
pub const USER_METADATA_MAX: usize = 2 * 1024;

/// Framing allowance on top of the decoded length when reading an aws-chunked body (headers, signatures, trailer).
pub const CHUNKED_FRAMING_ALLOWANCE: usize = 64 * 1024;

/// Bytes per verified block of a data file (1 MiB, ADR-0002): the unit a write hashes and a read verifies.
pub const DATA_BLOCK_SIZE: u32 = 1 << 20;

/// Largest object one PutObject may carry (5 GiB, S3's limit; EntityTooLarge beyond it).
pub const SINGLE_PUT_MAX: u64 = 5 * 1024 * 1024 * 1024;
