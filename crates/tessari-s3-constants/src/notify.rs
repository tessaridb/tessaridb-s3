//! How the delivery workers hold, retry and give up the events they send to notification destinations.

/// Seconds a delivery worker holds an event before the group hands it out again — longer than one delivery may take.
pub const NOTIFY_ACK_DEADLINE_SECS: u64 = 60;

/// Hand-outs of one event before it is given up and kept in the `object_events_dead` topic.
pub const NOTIFY_DELIVERIES: u32 = 8;

/// Events a destination's group lets be in flight at once, across every delivery worker.
pub const NOTIFY_IN_FLIGHT: usize = 64;

/// Events one delivery run reads at most: less than the group's width, so a worker that dies holding its events holds
/// only those until the ack deadline.
pub const NOTIFY_BATCH: usize = 16;

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
