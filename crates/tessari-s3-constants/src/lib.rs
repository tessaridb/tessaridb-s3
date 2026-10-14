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

/// Seconds an internal request's date may differ from the receiving node's clock: the window a captured request
/// can be replayed in, kept short because the nodes of one cluster share a time source.
pub const INTERNAL_MAX_SKEW_SECS: i64 = 5 * 60;

/// The internal shard routes: `{INTERNAL_SHARDS_PATH}/{data id, 32 lowercase hex}/{shard index}`.
pub const INTERNAL_SHARDS_PATH: &str = "/internal/v1/shards";

/// The internal route a node answers with 204 when the cluster asks whether it is up.
pub const INTERNAL_HEALTH_PATH: &str = "/internal/v1/health";

/// The internal route a node answers with its data drive's space.
pub const INTERNAL_DRIVE_PATH: &str = "/internal/v1/drive";

/// Largest drive answer read from a peer: three 20-digit numbers and two spaces.
pub const INTERNAL_DRIVE_BODY_MAX: usize = 64;

/// The header naming the node that sent an internal request.
pub const INTERNAL_NODE_HEADER: &str = "x-tessari-node";

/// The header carrying an internal request's date, in seconds since the Unix epoch.
pub const INTERNAL_DATE_HEADER: &str = "x-tessari-date";

/// The header carrying an internal request's signature, lowercase hex.
pub const INTERNAL_SIGNATURE_HEADER: &str = "x-tessari-signature";

/// Blocks queued for one shard before the encoder waits for its node: a stripe in flight and the next one.
pub const SHARD_QUEUE_BLOCKS: usize = 2;

/// Seconds a shard's node may keep its queue full before the shard is given up and the write goes on without it.
pub const SHARD_STALL_SECS: u64 = 30;

/// Seconds a node waits for a peer to accept a connection before calling it unavailable.
pub const PEER_CONNECT_TIMEOUT_SECS: u64 = 5;
/// How long a connection to the internal surface may take to finish its TLS handshake before it is dropped, so a
/// peer that connects and says nothing cannot hold a connection open.
pub const INTERNAL_TLS_HANDSHAKE_SECS: u64 = 10;

/// Seconds a node waits on a peer that has stopped sending mid-answer before calling it unavailable.
pub const PEER_READ_TIMEOUT_SECS: u64 = 30;

/// Shortest cluster secret accepted, in bytes: it keys HMAC-SHA256 over every internal request and the placement
/// hash, so it carries at least the 256 bits of the MAC it keys.
pub const CLUSTER_SECRET_MIN_LEN: usize = 32;

/// Seconds a connection to one metadata node may take to open before the next listed node is tried: long enough for
/// a TLS handshake across a data centre, short enough that a node behind a black-holed address does not hold a
/// request for the operating system's connect timeout.
pub const META_DIAL_TIMEOUT_SECS: u64 = 5;

/// New metadata connections signing in at once. Each spends the password on its first statement, an Argon2id check,
/// and a node runs at most 24 of those at once for EVERY client it has — past that it refuses the sign-in
/// (`SignInThrottled`) — so a burst on a cold pool waits here instead of being refused there.
pub const META_SIGN_IN_CONCURRENCY: usize = 4;

/// Seconds one unit of work may wait for the metadata store's answer once sent, when
/// `TESSARIDB_S3_META_STATEMENT_TIMEOUT_SECS` is unset: well past a failover round and a schema apply, and half the
/// node's own 60 s frame stall, so a node partitioned away mid-statement releases the request well before the
/// operating system would. A write that times out is unacknowledged, never failed: its data stays queued.
pub const DEFAULT_META_STATEMENT_TIMEOUT_SECS: u64 = 30;

/// Times the start-up schema is attempted when the store answers `retry` (another node is defining it at once).
pub const SCHEMA_RETRY_ATTEMPTS: u32 = 16;

/// Milliseconds of back-off per attempt between schema retries (20, 40, 60 … ms).
pub const SCHEMA_RETRY_BACKOFF_MS: u64 = 20;

/// Times a node proposes the first layout when the store answers `retry` (another proposal committed after this
/// one's snapshot): the next attempt sees the committed layout and reads it back.
pub const LAYOUT_PROPOSE_ATTEMPTS: u32 = 3;

/// Times an action is appended to the action record when the store answers `retry` (another action committed to the
/// record after this one's snapshot); a refused append wrote nothing, so the next attempt cannot duplicate it.
pub const ACTION_RECORD_ATTEMPTS: u32 = 5;

/// Milliseconds of back-off per attempt between action-record appends (20, 40, 60 … ms, plus a spread): without it
/// concurrent appends that lost once lose again together.
pub const ACTION_RECORD_BACKOFF_MS: u64 = 20;

/// Largest sum of user metadata names and values, in UTF-8 bytes (2 KB, S3's limit; MetadataTooLarge beyond it).
pub const USER_METADATA_MAX: usize = 2 * 1024;

/// Framing allowance on top of the decoded length when reading an aws-chunked body (headers, signatures, trailer).
pub const CHUNKED_FRAMING_ALLOWANCE: usize = 64 * 1024;

/// Bytes per verified block of a data file (1 MiB, ADR-0002): the unit a write hashes and a read verifies.
pub const DATA_BLOCK_SIZE: u32 = 1 << 20;

/// The most shards one object is cut into (k+m ≤ 16): each shard goes to a distinct node, and a set wider than this
/// buys nothing a cluster of this size can place.
pub const ERASURE_MAX_WIDTH: u8 = 16;

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

/// Seconds between healing passes on a cluster member, when `TESSARIDB_S3_HEAL_INTERVAL_SECS` is unset.
pub const DEFAULT_HEAL_INTERVAL_SECS: u64 = 60;

/// Seconds a console session lasts from sign-in; there is no refresh — the operator signs in again.
pub const CONSOLE_SESSION_SECS: i64 = 3_600;

/// The cookie the console session travels in.
pub const CONSOLE_SESSION_COOKIE: &str = "tessaridb_s3_console";

/// Sign-in attempts accepted per client address per minute, when `TESSARIDB_S3_CONSOLE_SIGN_INS_PER_MINUTE` is unset:
/// enough for an operator who mistypes, far too few to guess a secret.
pub const DEFAULT_CONSOLE_SIGN_INS_PER_MINUTE: usize = 10;

/// Console API requests accepted per signed-in key per minute, when `TESSARIDB_S3_CONSOLE_REQUESTS_PER_MINUTE` is
/// unset: a page refreshing every few seconds across several tabs fits well under it.
pub const DEFAULT_CONSOLE_REQUESTS_PER_MINUTE: usize = 600;

/// The most console actions one page of the action record answers.
pub const CONSOLE_ACTIONS_PAGE_MAX: usize = 100;

/// The longest reason, in characters, an operator may give for a console action.
pub const CONSOLE_REASON_MAX_CHARS: usize = 500;

/// Seconds a console key-issue token is remembered: a repeated submission inside this window answers the key the
/// token first issued and never a second secret; past it the token is forgotten and a submission issues anew.
pub const KEY_ISSUE_TOKEN_SECS: i64 = 86_400;

/// The most access keys one user's console listing returns.
pub const USER_KEYS_LISTED_MAX: usize = 1_000;

/// The shortest life, in seconds, of a one-key upload credential: long enough to be used, and an expiry the metadata
/// store will accept as still in the future.
pub const UPLOAD_KEY_MIN_SECS: i64 = 60;

/// The longest life, in seconds, of a one-key upload credential: seven days, as for a presigned URL.
pub const UPLOAD_KEY_MAX_SECS: i64 = 604_800;

/// How far the console counts the heal backlog before it says "more".
pub const CONSOLE_BACKLOG_COUNT_MAX: usize = 1_000;

/// Seconds the console waits for a member to answer before it shows that member as not answering.
pub const CONSOLE_MEMBER_PROBE_SECS: u64 = 2;

/// Object keys (and common prefixes) on a console listing page when the request does not say.
pub const CONSOLE_OBJECTS_PAGE_DEFAULT: usize = 100;

/// Data ids one healing pass looks at.
pub const HEAL_BATCH: usize = 100;

/// Seconds a node's claim on one data id lasts — taken by a healer before it checks and rewrites shards, and by a
/// reclaimer before it removes them, so one node at a time works on an id. A lease, not a mutex: past it another node
/// may take the id, which costs duplicate work (two healers end in one refusal), never a wrong shard, because a
/// verified shard is never rewritten and a second writer of one shard is refused. It is also how long a node that
/// died holding a claim, and does not come back, keeps the others off that id.
pub const HEAL_CLAIM_SECS: u64 = 120;

/// Hand-outs after which a data id still listed for healing is reported at risk: a heal that failed this many times
/// will not succeed by itself.
pub const HEAL_AT_RISK_ATTEMPTS: i64 = 10;

/// Seconds a data id whose heal did not finish waits before it is handed out again, so it leaves the head of the
/// queue.
pub const HEAL_RETRY_SECS: u64 = 15;

/// How long a member's claim on the usage pass lasts. Longer than the default interval between passes, so the member
/// that measured keeps measuring; when it stops, another member takes the pass once the claim lapses.
pub const USAGE_CLAIM_SECS: u64 = 180;

/// Default time between usage passes (`TESSARIDB_S3_USAGE_INTERVAL_SECS`). A pass reads every object record, so the
/// figures the console shows are at most this old.
pub const DEFAULT_USAGE_INTERVAL_SECS: u64 = 60;

/// How many usage intervals a measurement counts for when a write is checked against a bucket quota. One missed
/// pass is tolerated; past two intervals a limited bucket's writes are refused rather than admitted on a figure
/// nobody refreshed.
pub const QUOTA_MEASUREMENT_INTERVALS: u64 = 2;

/// Seconds a write may spend counting one limited bucket when its usage measurement is too old. Past it the count
/// refuses rather than answering in part, and the write is refused as unavailable.
pub const QUOTA_COUNT_TIMEOUT_SECS: u64 = 2;

/// Queue entries one reclamation pass looks at.
pub const RECLAIM_BATCH: usize = 1000;

/// Seconds a queued data id a pass could not finish (another node holds its shards) waits before it is handed out
/// again, so it leaves the head of the queue.
pub const RECLAIM_RETRY_SECS: u64 = 15;

/// Buckets one lifecycle pass claims at most.
pub const LIFECYCLE_BATCH: usize = 100;

/// Seconds after a lifecycle pass before the same bucket is due again. A rule's period is whole days rounded to the
/// next day boundary, so an hour late is the most an action waits.
pub const LIFECYCLE_INTERVAL_SECS: u64 = 3600;

/// Seconds a delivery worker holds an event before the group hands it out again — longer than one delivery may take.
pub const NOTIFY_ACK_DEADLINE_SECS: u64 = 60;

/// Hand-outs of one event before it is given up and kept in the `object_events_dead` topic.
pub const NOTIFY_DELIVERIES: u32 = 8;

/// Events a destination's group holds in flight at once.
pub const NOTIFY_IN_FLIGHT: usize = 64;

/// Events one delivery run reads at most.
pub const NOTIFY_BATCH: usize = 64;

/// Deliveries one run sends at once to its destination.
pub const NOTIFY_CONCURRENCY: usize = 8;

/// Seconds a delivery may take before it counts as failed.
pub const NOTIFY_DELIVERY_TIMEOUT_SECS: u64 = 10;

/// Seconds before a failed delivery is tried again after its first hand-out; doubled per hand-out, at most
/// [`NOTIFY_RETRY_MAX_SECS`].
pub const NOTIFY_RETRY_STEP_SECS: u64 = 2;

/// Longest wait between two tries of one delivery.
pub const NOTIFY_RETRY_MAX_SECS: u64 = 300;

/// Seconds between delivery runs when the last found nothing.
pub const NOTIFY_POLL_SECS: u64 = 1;

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

/// Attempts of one write or delete of a key before giving up when another writer of the same key commits first every
/// time; each attempt reads the key's newest version again (ADR-0010). One writer wins each round, so with n writers of
/// one key the last of them needs n attempts; past this bound the request is answered as contended (503 SlowDown).
pub const VERSION_WRITE_ATTEMPTS: u32 = 32;

/// Most parts one ListParts page carries, and the default.
pub const LIST_PARTS_MAX: usize = 1000;

/// Most uploads (and common prefixes) one ListMultipartUploads page returns.
pub const LIST_UPLOADS_MAX: usize = 1000;

/// The space every bucket created before spaces existed belongs to, and the one the root credential creates in.
pub const DEFAULT_SPACE: &str = "default";

/// Object records still holding their bytes (a store from before ADR-0008) converted per batch at start.
pub const INLINE_CONVERT_BATCH: usize = 100;

/// Object records read per batch while a store from before ADR-0010 gets its null versions at start.
pub const VERSION_CONVERT_BATCH: usize = 100;

/// The size a source must exceed for UploadPartCopy to copy a byte range of it. AWS documents "greater than 5 MB";
/// read as MiB, the unit of the part minimum it sits beside (inferred — the page does not say which).
pub const RANGE_COPY_SOURCE_MIN: u64 = 5 * 1024 * 1024;
