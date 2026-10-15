//! Rendition tasks: queue each output of a version once, hand tasks to a node's workers, and settle each by its
//! outcome — removed when written, handed back after a growing wait when not, and left in its queue as given up once
//! its hand-outs are spent.

use tessari_s3_constants::{MEDIA_RETRY_MAX_SECS, MEDIA_RETRY_STEP_SECS, MEDIA_TASK_ATTEMPTS};

use super::model::{Rendition, Task, TaskQueue};
use super::repository::TaskRepository;
use super::service::MediaService;
use crate::Result;
use crate::topic_groups;

impl MediaService {
    /// Queues `rendition` in the queue its output needs; queuing the same output of the same version again is a no-op.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn enqueue(&self, rendition: &Rendition) -> Result<()> {
        self.repository.enqueue(rendition).await
    }

    /// Holds up to `limit` tasks of `queue` for `consumer` (letters, digits, `-` or `_`); none when another worker
    /// won the race for them.
    ///
    /// # Errors
    /// A malformed consumer name, or the metadata store's refusal or outage.
    pub async fn claim(&self, queue: TaskQueue, consumer: &str, limit: usize) -> Result<Vec<Task>> {
        self.repository.claim(queue, consumer, limit).await
    }

    /// Settles `task`: its rendition is written, or the version it names is gone.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn rendered(&self, task: &Task) -> Result<()> {
        self.repository.remove(task).await
    }

    /// Records why `task` failed and hands it back after a wait that doubles with its hand-outs.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn retry(&self, task: &Task, error: &str) -> Result<()> {
        let delay = topic_groups::retry_after(
            u64::try_from(task.attempts).unwrap_or(0),
            MEDIA_RETRY_STEP_SECS,
            MEDIA_RETRY_MAX_SECS,
        );
        self.repository.release(task, error, delay).await
    }

    /// Frees every task of `queue` held under `consumer` — what a node that restarted under its own name left held.
    ///
    /// # Errors
    /// A malformed consumer name, or the metadata store's refusal or outage.
    pub async fn reclaim(&self, queue: TaskQueue, consumer: &str) -> Result<()> {
        self.repository.release_all(queue, consumer).await
    }

    /// Up to `limit` tasks of `queue` whose hand-outs are spent: no worker takes them again.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn given_up(&self, queue: TaskQueue, limit: usize) -> Result<Vec<Task>> {
        self.repository
            .spent(queue, MEDIA_TASK_ATTEMPTS, limit)
            .await
    }
}
