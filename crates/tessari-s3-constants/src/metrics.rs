//! Reading the backlogs a metrics scrape reports.

/// How many ids a metrics scrape counts the heal backlog up to.
pub const METRICS_BACKLOG_COUNT_MAX: usize = 1_000;

/// Seconds a metrics scrape waits for each backlog it reads before reporting that source down.
pub const METRICS_COLLECT_SECS: u64 = 5;
