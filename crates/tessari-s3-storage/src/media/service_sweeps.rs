//! Sweeps: a bucket's existing objects planned a page at a time, so renditions reach objects written before a rule.

use tessari_s3_core::media::Rule;
use tessari_s3_types::BucketName;

use super::model::{Sweep, SweepProgress};
use super::repository::SweepRepository;
use super::service::MediaService;
use crate::Result;

impl MediaService {
    /// Starts `bucket`'s sweep from its first key; one under way starts over.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn start_sweep(&self, bucket: &BucketName) -> Result<()> {
        self.repository.start(bucket.as_str()).await
    }

    /// Holds the next sweep with keys left and the media rules of its bucket as they stand now (none when the bucket
    /// or its rules are gone).
    ///
    /// # Errors
    /// The metadata store's refusal or outage, or a malformed record.
    pub async fn next_sweep(&self) -> Result<Option<(Sweep, Vec<Rule>)>> {
        let Some(sweep) = self.repository.claim().await? else {
            return Ok(None);
        };
        let rules = match BucketName::new(&sweep.bucket) {
            Ok(name) => self
                .buckets
                .get(&name)
                .await?
                .and_then(|bucket| bucket.media)
                .unwrap_or_default(),
            Err(_) => Vec::new(),
        };
        Ok(Some((sweep, rules)))
    }

    /// Records `sweep` planned up to its `after`, finished when `last`; `false` when it was started again meanwhile.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn advance_sweep(&self, sweep: &Sweep, last: bool) -> Result<bool> {
        self.repository.advance(sweep, last).await
    }

    /// Where `bucket`'s sweep stands.
    ///
    /// # Errors
    /// The metadata store's refusal or outage, or a malformed record.
    pub async fn sweep_progress(&self, bucket: &BucketName) -> Result<SweepProgress> {
        self.repository.progress(bucket.as_str()).await
    }
}
