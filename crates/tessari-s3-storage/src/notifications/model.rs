//! An object change as a destination receives it.

use tessari_s3_types::Timestamp;

/// One change in a bucket that notifies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// Its topic position — the S3 `sequencer`, ordering the changes of one key and every other.
    pub position: u64,
    /// How many times it has been handed out, this time included.
    pub deliveries: u64,
    /// The bucket.
    pub bucket: String,
    /// The object key.
    pub key: String,
    /// The full event name, such as `s3:ObjectCreated:Put`.
    pub name: String,
    /// The version it concerns, where the bucket keeps versions.
    pub version_id: Option<String>,
    /// The object's size, for a created object.
    pub size: Option<u64>,
    /// The object's ETag, for a created object.
    pub etag: Option<String>,
    /// When the change happened.
    pub at: Timestamp,
}

/// An event one of the destination's rules matches, to be delivered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Due {
    /// The event.
    pub event: Event,
    /// The id of the first rule that matched it — the S3 `configurationId`.
    pub configuration: String,
}
