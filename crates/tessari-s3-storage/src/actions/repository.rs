//! What the action record must answer, whatever stores it.

use std::future::Future;

use super::entity::ActionEntity;
use super::model::NewAction;
use crate::Result;

/// Action-record queries; the record only grows.
pub(crate) trait ActionRepository: Send + Sync {
    /// Appends `action`, stamped with the store's clock.
    fn append(&self, action: &NewAction) -> impl Future<Output = Result<()>> + Send;
    /// The position of the newest action; `0` when none was ever recorded.
    fn last(&self) -> impl Future<Output = Result<u64>> + Send;
    /// At most `limit` actions after position `after`, oldest first.
    fn after(
        &self,
        after: u64,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<ActionEntity>>> + Send;
}
