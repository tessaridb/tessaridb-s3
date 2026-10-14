//! What the media group must answer, whatever stores it.

use std::future::Future;

use crate::Result;
use crate::topic_groups::MessageEntity;

/// Topic-group queries for the `media` group.
pub(crate) trait MediaRepository: Send + Sync {
    /// Declares the group when it is missing; a new group starts at the topic's oldest message.
    fn define(&self) -> impl Future<Output = Result<()>> + Send;
    /// Hands out up to `limit` messages.
    fn read(&self, limit: usize) -> impl Future<Output = Result<Vec<MessageEntity>>> + Send;
    /// Settles `positions` for good.
    fn ack(&self, positions: &[u64]) -> impl Future<Output = Result<()>> + Send;
    /// Hands `positions` out again no sooner than `delay_secs` from now.
    fn nack(&self, positions: &[u64], delay_secs: u64) -> impl Future<Output = Result<()>> + Send;
}
