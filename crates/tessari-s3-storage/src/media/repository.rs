//! What the media group and the rendition queues must answer, whatever stores them.

use std::future::Future;

use super::model::{Rendition, Sweep, SweepProgress, Task, TaskQueue};
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

/// Rendition-task queries over `media_images` and `media_videos`.
pub(crate) trait TaskRepository: Send + Sync {
    /// Queues `rendition` once: a second queuing of the same output of the same version within the queue's window
    /// writes nothing.
    fn enqueue(&self, rendition: &Rendition) -> impl Future<Output = Result<()>> + Send;
    /// Holds up to `limit` tasks of `queue` under `consumer`.
    fn claim(
        &self,
        queue: TaskQueue,
        consumer: &str,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<Task>>> + Send;
    /// Removes `task`: its rendition is written, or it needs none.
    fn remove(&self, task: &Task) -> impl Future<Output = Result<()>> + Send;
    /// Records `error` on `task` and hands it back, not to be taken again for `delay_secs`.
    fn release(
        &self,
        task: &Task,
        error: &str,
        delay_secs: u64,
    ) -> impl Future<Output = Result<()>> + Send;
    /// Frees every task of `queue` any session holds under `consumer`.
    fn release_all(
        &self,
        queue: TaskQueue,
        consumer: &str,
    ) -> impl Future<Output = Result<()>> + Send;
    /// Up to `limit` tasks of `queue` handed out at least `attempts` times.
    fn spent(
        &self,
        queue: TaskQueue,
        attempts: i64,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<Task>>> + Send;
}

/// Sweep queries over `media_sweeps` and `media_swept`.
pub(crate) trait SweepRepository: Send + Sync {
    /// Starts `bucket`'s sweep from its first key, over one still under way.
    fn start(&self, bucket: &str) -> impl Future<Output = Result<()>> + Send;
    /// Holds the next sweep with keys left, if one has.
    fn claim(&self) -> impl Future<Output = Result<Option<Sweep>>> + Send;
    /// Records `sweep` planned up to its `after` and hands it back, or, when `last`, records it finished; `false`
    /// when it was started again meanwhile and nothing was recorded.
    fn advance(&self, sweep: &Sweep, last: bool) -> impl Future<Output = Result<bool>> + Send;
    /// Where `bucket`'s sweep stands.
    fn progress(&self, bucket: &str) -> impl Future<Output = Result<SweepProgress>> + Send;
}
