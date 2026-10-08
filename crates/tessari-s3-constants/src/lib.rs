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

/// Seconds a connection to one metadata node may take to open before the next listed node is tried: long enough for
/// a TLS handshake across a data centre, short enough that a node behind a black-holed address does not hold a
/// request for the operating system's connect timeout.
pub const META_DIAL_TIMEOUT_SECS: u64 = 5;

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

/// Seconds a queued data file is kept before the reclaimer may remove it (24 h), when
/// `TESSARIDB_S3_RECLAIM_GRACE_SECS` is unset: longer than any upload and any read in flight.
pub const DEFAULT_RECLAIM_GRACE_SECS: u64 = 24 * 60 * 60;

/// Seconds between reclamation passes, when `TESSARIDB_S3_RECLAIM_INTERVAL_SECS` is unset.
pub const DEFAULT_RECLAIM_INTERVAL_SECS: u64 = 300;

/// Seconds a multipart upload may stay open before the reaper aborts it (7 days), when
/// `TESSARIDB_S3_UPLOAD_MAX_AGE_SECS` is unset: the server-wide backstop for uploads nobody finishes.
pub const DEFAULT_UPLOAD_MAX_AGE_SECS: u64 = 7 * 24 * 60 * 60;

/// Uploads one reaper pass aborts at most.
pub const REAP_BATCH: usize = 1000;

/// Queue entries one reclamation pass looks at.
pub const RECLAIM_BATCH: usize = 1000;

/// Longest one daemon run may take before it is abandoned and reported.
pub const DAEMON_RUN_TIMEOUT_SECS: u64 = 300;

/// Most entries (keys and common prefixes) one ListObjects page carries, and the default.
pub const LIST_MAX_KEYS: usize = 1000;

/// Most keys one DeleteObjects request may name.
pub const DELETE_OBJECTS_MAX_KEYS: usize = 1000;

/// The largest DeleteObjects body read: the maximum keys, each at the key limit with every byte escaped (`&quot;`
/// is six bytes) plus its element tags.
pub const DELETE_OBJECTS_BODY_MAX: usize = DELETE_OBJECTS_MAX_KEYS * (OBJECT_KEY_MAX_LEN * 6 + 128);

/// Keys of one DeleteObjects request removed at once; well under the metadata connection pool.
pub const DELETE_OBJECTS_CONCURRENCY: usize = 8;

/// Highest part number, and the most parts one multipart upload holds.
pub const MULTIPART_MAX_PARTS: u16 = 10_000;

/// Smallest size of every part but the last of a multipart upload, checked at completion.
pub const MULTIPART_MIN_PART_SIZE: u64 = 5 * 1024 * 1024;

/// Largest CompleteMultipartUpload body read: 10,000 parts at 512 bytes each, room for a part number, an ETag and
/// every checksum element.
pub const COMPLETE_MULTIPART_BODY_MAX: usize = 10_000 * 512;

/// Reads of a multipart object's key before giving up when it is overwritten between reading the object and its parts
/// every time.
pub const MULTIPART_READ_ATTEMPTS: u32 = 3;

/// Most parts one ListParts page carries, and the default.
pub const LIST_PARTS_MAX: usize = 1000;

/// Most uploads (and common prefixes) one ListMultipartUploads page returns.
pub const LIST_UPLOADS_MAX: usize = 1000;
