//! How the replicators hold, retry and give up the versions they send (S3 ref 11 M4).

/// Seconds a replicator holds a replication task before the group hands it out again — longer than sending one
/// version may take.
pub const REPLICATION_ACK_DEADLINE_SECS: u64 = 300;

/// Hand-outs of one replication task before it is given up: its version is stamped `FAILED` and the task kept in the
/// `replication_failed` topic.
pub const REPLICATION_DELIVERIES: u32 = 10;

/// Replication tasks the group holds in flight at once.
pub const REPLICATION_IN_FLIGHT: usize = 4;

/// Seconds before a failed replication is tried again after its first hand-out; doubled per hand-out, at most
/// [`REPLICATION_RETRY_MAX_SECS`].
pub const REPLICATION_RETRY_STEP_SECS: u64 = 5;

/// Longest wait between two tries of one replication.
pub const REPLICATION_RETRY_MAX_SECS: u64 = 600;

/// Seconds a replicator waits to connect to a destination region's S3 surface; the transfer itself is bounded by the
/// group's hold ([`REPLICATION_ACK_DEADLINE_SECS`]).
pub const REPLICATION_CONNECT_TIMEOUT_SECS: u64 = 10;

/// Seconds between replication runs when the last found nothing.
pub const REPLICATION_POLL_SECS: u64 = 1;
