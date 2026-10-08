//! An operator action as the rest of the server writes and reads it.

use tessari_s3_types::Timestamp;

use super::entity::ActionEntity;
use crate::{Error, Result};

/// One action to record. Identifiers only: a target names a bucket or an object, never what the object holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewAction {
    /// The key id the operator signed in with.
    pub operator: String,
    /// The operation by its domain name (`delete_object` …).
    pub operation: String,
    /// What it was done to: `bucket` or `bucket/key`.
    pub target: String,
    /// Why, in the operator's words; required by the console on every action that removes or reveals data.
    pub reason: Option<String>,
    /// How it ended (`done`, `precondition_failed` …).
    pub outcome: String,
}

/// One recorded action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    /// Its place in the record: dense, in commit order.
    pub position: u64,
    /// When it was recorded, by the metadata store's clock.
    pub at: Timestamp,
    /// The key id the operator signed in with.
    pub operator: String,
    /// The operation by its domain name.
    pub operation: String,
    /// What it was done to.
    pub target: String,
    /// Why.
    pub reason: Option<String>,
    /// How it ended.
    pub outcome: String,
}

impl TryFrom<ActionEntity> for Action {
    type Error = Error;

    fn try_from(entity: ActionEntity) -> Result<Self> {
        let (seconds, nanos) = entity.at;
        Ok(Self {
            position: u64::try_from(entity.position).map_err(|_| Error::Malformed {
                record: "console_actions",
                reason: "position",
            })?,
            at: Timestamp { seconds, nanos },
            operator: entity.operator,
            operation: entity.operation,
            target: entity.target,
            reason: entity.reason,
            outcome: entity.outcome,
        })
    }
}
