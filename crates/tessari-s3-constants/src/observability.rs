//! What a node tells about itself: its per-request access log — how many records wait for the writer, how a batch
//! is bounded, how a failed append is retried — and how a metrics scrape reads the backlogs it reports.

/// Records waiting for the access-log writer; a request arriving past it is counted as dropped and still served.
pub const AUDIT_QUEUE_MAX: usize = 10_000;

/// Records the writer appends in one transaction at most.
pub const AUDIT_BATCH_ROWS: usize = 500;

/// Estimated bytes the writer appends in one transaction at most.
pub const AUDIT_BATCH_BYTES: usize = 1 << 20;

/// Times a batch is tried before its records are counted as failed.
pub const AUDIT_APPEND_ATTEMPTS: u32 = 3;

/// Milliseconds before a failed append is tried again, doubled each time.
pub const AUDIT_RETRY_MS: u64 = 200;

/// How many ids a metrics scrape counts the heal backlog up to.
pub const METRICS_BACKLOG_COUNT_MAX: usize = 1_000;

/// Seconds a metrics scrape waits for each backlog it reads before reporting that source down.
pub const METRICS_COLLECT_SECS: u64 = 5;
