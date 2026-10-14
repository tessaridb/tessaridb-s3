//! What a destination's group must answer, whatever stores it.

use std::future::Future;

use crate::Result;
use crate::topic_groups::MessageEntity;

/// Topic-group queries for the destination `target`.
pub(crate) trait NotificationRepository: Send + Sync {
    /// Declares `target`'s group when it is missing; a new group starts at the topic's oldest message.
    fn define(&self, target: &str) -> impl Future<Output = Result<()>> + Send;
    /// Hands out up to `limit` messages to `target`'s group.
    fn read(
        &self,
        target: &str,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<MessageEntity>>> + Send;
    /// Appends `body` (JSON) to `s3_notifications` as `n-<position>`; an append already made is left as it is.
    fn publish(&self, position: u64, body: &str) -> impl Future<Output = Result<()>> + Send;
    /// Settles `positions` for good.
    fn ack(&self, target: &str, positions: &[u64]) -> impl Future<Output = Result<()>> + Send;
    /// Hands `positions` out again no sooner than `delay_secs` from now.
    fn nack(
        &self,
        target: &str,
        positions: &[u64],
        delay_secs: u64,
    ) -> impl Future<Output = Result<()>> + Send;
}
