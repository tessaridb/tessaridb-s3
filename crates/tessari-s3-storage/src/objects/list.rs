//! Listing a bucket: the core lister fed batches of object records read in record-id order from the bucket index.
//! Only the bucket's current incarnation is shown, and the walk stops at the first record of another bucket.

use tessari_s3_constants::LIST_MAX_KEYS;
use tessari_s3_core::objects::listing::{Anchor, ListSpec, Lister};
use tessari_s3_types::BucketName;

use super::ObjectService;
use crate::Result;
use crate::objects::model::Listed;
use crate::objects::repository::ObjectRepository;

impl ObjectService {
    /// One listing page of `bucket` as `spec` asks for it.
    ///
    /// # Errors
    /// The metadata store's refusal or outage, or a malformed record.
    pub async fn list(&self, bucket: &BucketName, spec: ListSpec) -> Result<Listed> {
        // One more than the page so the peek that decides IsTruncated usually costs no second read.
        let batch_size = spec.max_keys.saturating_add(1).clamp(1, LIST_MAX_KEYS);
        let mut lister = Lister::new(spec);
        let mut current = None;
        // Where to continue when a whole batch held nothing visible (records of an earlier incarnation): the lister
        // saw nothing, so its anchor did not move, and this cursor does.
        let mut past_invisible: Option<String> = None;
        loop {
            let (at, after) = match (past_invisible.take(), lister.anchor()) {
                (Some(key), _) => (None, key),
                (None, None) => break,
                (None, Some(Anchor::After(key))) => (None, key.clone()),
                (None, Some(Anchor::At(key))) => (Some(key.clone()), key.clone()),
            };
            let batch = self
                .repository
                .page(bucket, at.as_deref(), &after, batch_size)
                .await?;
            let Some(now) = batch.bucket else {
                return Ok(Listed::NoSuchBucket);
            };
            // The incarnation the listing started in holds for every page of it.
            let incarnation = *current.get_or_insert(now);
            let mut more = batch.full;
            let mut visible = Vec::with_capacity(batch.rows.len());
            let mut last = None;
            for row in batch.rows {
                if row.bucket != bucket.as_str() {
                    more = false;
                    break;
                }
                if row.incarnation == incarnation {
                    visible.push((row.key, row.summary));
                } else {
                    last = Some(row.key);
                }
            }
            if visible.is_empty() && more {
                past_invisible = last;
                if past_invisible.is_none() {
                    break;
                }
                continue;
            }
            lister.feed(visible, more);
        }
        Ok(Listed::Page(lister.finish()))
    }
}
