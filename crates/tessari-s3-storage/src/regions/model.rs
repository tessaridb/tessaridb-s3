//! What a region's failover record says, and how a failover or a restore ended.

use tessari_s3_types::Timestamp;

/// One region's failover record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionState {
    /// The region.
    pub region: String,
    /// How many failovers and restores the region has had; a node holding an older one is behind.
    pub epoch: u64,
    /// While failed over, the region its writes go to; `None` once restored.
    pub promoted: Option<String>,
    /// The operator's reason for the last change.
    pub reason: String,
    /// Who made it.
    pub by: String,
    /// When.
    pub at: Timestamp,
}

impl RegionState {
    /// Whether the region is failed over, and so refuses client writes.
    #[must_use]
    pub const fn failed_over(&self) -> bool {
        self.promoted.is_some()
    }
}

/// How a failover ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailOver {
    /// The region is failed over under the next epoch.
    FailedOver(RegionState),
    /// It already was; nothing changed.
    AlreadyFailedOver(RegionState),
    /// A region is never failed over to itself.
    ToItself,
    /// Another change committed between the read and this one; nothing changed.
    Contended,
}

/// How a restore ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Restore {
    /// The region takes client writes again, under the next epoch.
    Restored(RegionState),
    /// It was not failed over; nothing changed.
    NotFailedOver,
    /// Another change committed between the read and this one; nothing changed.
    Contended,
}
