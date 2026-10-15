//! A replication task as a replicator receives it.

/// One version to send to its destination bucket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    /// Its topic position.
    pub position: u64,
    /// How many times it has been handed out, this time included.
    pub deliveries: u64,
    /// The source bucket.
    pub bucket: String,
    /// The object key.
    pub key: String,
    /// The version's id, which the replica keeps.
    pub version_id: String,
    /// The version's sequence among the key's versions.
    pub seq: u64,
    /// Whether the version is a delete marker.
    pub marker: bool,
    /// The bucket it is sent to.
    pub destination: String,
}

/// Where this region's replication stands, as its topic reports it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Progress {
    /// Tasks not yet settled: queued and not handed out, in flight, or waiting to be tried again.
    pub backlog: u64,
    /// Tasks handed out and not yet settled.
    pub in_flight: u64,
    /// Tasks given up: their versions are `FAILED`.
    pub failed: u64,
}
