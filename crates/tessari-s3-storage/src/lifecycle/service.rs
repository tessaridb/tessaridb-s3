//! Lifecycle passes: claim the buckets due, apply their rules, and hand each back due again an interval later.

use tessari_s3_types::{BucketName, Timestamp};

use super::entity::RunEntity;
use super::model::Applied;
use super::repository::LifecycleRepository;
use super::tessaridb::TessariLifecycle;
use crate::Result;
use crate::buckets::BucketService;
use crate::multipart::MultipartService;
use crate::objects::ObjectService;

/// Lifecycle operations.
#[derive(Clone)]
pub struct LifecycleService {
    repository: TessariLifecycle,
    pub(super) buckets: BucketService,
    pub(super) objects: ObjectService,
    pub(super) multipart: MultipartService,
}

impl LifecycleService {
    /// The engine over `repository`, acting through the bucket, object and multipart services.
    #[must_use]
    pub const fn new(
        repository: TessariLifecycle,
        buckets: BucketService,
        objects: ObjectService,
        multipart: MultipartService,
    ) -> Self {
        Self {
            repository,
            buckets,
            objects,
            multipart,
        }
    }

    /// One pass at the instant `now`: up to `batch` buckets that are due have their rules applied and are handed back
    /// due `interval_secs` later. A bucket whose rules are gone, or that is gone, leaves the queue. A bucket left
    /// claimed by an error is handed out again once its hold lapses.
    ///
    /// # Errors
    /// The metadata store's refusal or outage, or a malformed record.
    pub async fn pass(&self, now: Timestamp, batch: usize, interval_secs: u64) -> Result<Applied> {
        let mut applied = Applied::default();
        let mut seen: Vec<String> = Vec::new();
        while seen.len() < batch {
            let Some(run) = self.repository.claim().await? else {
                break;
            };
            // Handed out again in this pass (a short interval): it is done for now.
            if seen.contains(&run.bucket) {
                self.repository.release(&run.bucket, interval_secs).await?;
                break;
            }
            seen.push(run.bucket.clone());
            self.run(&run, now, interval_secs, &mut applied).await?;
        }
        Ok(applied)
    }

    /// Applies one claimed bucket's rules and hands it back, or takes it off the queue when it has none.
    async fn run(
        &self,
        run: &RunEntity,
        now: Timestamp,
        interval_secs: u64,
        applied: &mut Applied,
    ) -> Result<()> {
        let rules = match BucketName::new(&run.bucket) {
            Ok(name) => self
                .buckets
                .get(&name)
                .await?
                .and_then(|bucket| bucket.lifecycle)
                .filter(|rules| !rules.is_empty())
                .map(|rules| (name, rules)),
            Err(_) => None,
        };
        let Some((bucket, rules)) = rules else {
            if !self.repository.forget(run).await? {
                // The rules were written again while it was claimed: their record stays, free for the next pass.
                self.repository.release(&run.bucket, 0).await?;
            }
            return Ok(());
        };
        self.apply(&bucket, &rules, now, applied).await?;
        applied.buckets = applied.buckets.saturating_add(1);
        self.repository.release(&run.bucket, interval_secs).await
    }
}
