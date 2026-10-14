//! An `object_events` message exactly as a group hands it out.

use tessari_s3_infrastructure::tessaridb::Value;

/// One handed-out message.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MessageEntity {
    /// Its topic position: dense and commit-ordered.
    pub position: u64,
    /// How many times the group has handed it out, this time included.
    pub deliveries: u64,
    /// The message as the store's event wrote it.
    pub value: Value,
}
