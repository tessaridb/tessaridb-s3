//! Media job bookkeeping: hand out the next created objects, keep the ones a rule of their bucket's CURRENT media
//! rules may take, settle the rest at once, and settle or retry the taken ones by their outcome.

use std::collections::BTreeMap;

use tessari_s3_constants::{MEDIA_RETRY_MAX_SECS, MEDIA_RETRY_STEP_SECS};
use tessari_s3_core::media::{RENDITIONS, Rule};
use tessari_s3_types::BucketName;

use super::model::Job;
use super::repository::MediaRepository;
use super::tessaridb::TessariMedia;
use crate::Result;
use crate::buckets::BucketService;
use crate::topic_groups::{self, event};

/// The event family a rendition is made for.
const CREATED: &str = "s3:ObjectCreated:";

/// Media job operations.
#[derive(Clone)]
pub struct MediaService {
    repository: TessariMedia,
    buckets: BucketService,
}

impl MediaService {
    /// Media jobs over `repository`, taking rules through `buckets`.
    #[must_use]
    pub const fn new(repository: TessariMedia, buckets: BucketService) -> Self {
        Self {
            repository,
            buckets,
        }
    }

    /// Declares the media group when it is missing.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn define(&self) -> Result<()> {
        self.repository.define().await
    }

    /// The next jobs (at most `limit` handed out).
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn next(&self, limit: usize) -> Result<Vec<Job>> {
        let mut rules_of: BTreeMap<String, Vec<Rule>> = BTreeMap::new();
        let (mut jobs, mut unmatched) = (Vec::new(), Vec::new());
        for message in self.repository.read(limit).await? {
            let (position, deliveries) = (message.position, message.deliveries);
            let event = match event(message) {
                Ok(event) => event,
                Err(error) => {
                    // Handed back rather than settled, so the dead letter keeps it instead of it vanishing.
                    tracing::warn!(position, %error, "an event message cannot be read");
                    let delay = topic_groups::retry_after(
                        deliveries,
                        MEDIA_RETRY_STEP_SECS,
                        MEDIA_RETRY_MAX_SECS,
                    );
                    self.repository.nack(&[position], delay).await?;
                    continue;
                }
            };
            if !rules_of.contains_key(&event.bucket) {
                let rules = match BucketName::new(&event.bucket) {
                    Ok(name) => self
                        .buckets
                        .get(&name)
                        .await?
                        .and_then(|bucket| bucket.media)
                        .unwrap_or_default(),
                    Err(_) => Vec::new(),
                };
                rules_of.insert(event.bucket.clone(), rules);
            }
            let rules = rules_of.get(&event.bucket).cloned().unwrap_or_default();
            // The content type is the object's, read by the worker; here only what the event alone decides.
            let taken = event.name.starts_with(CREATED)
                && !event.key.contains(RENDITIONS)
                && rules.iter().any(|rule| event.key.starts_with(&rule.prefix));
            if taken {
                jobs.push(Job { event, rules });
            } else {
                unmatched.push(event.position);
            }
        }
        self.repository.ack(&unmatched).await?;
        Ok(jobs)
    }

    /// Settles `job`: its renditions are written, or it needs none.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn done(&self, job: &Job) -> Result<()> {
        self.repository.ack(&[job.event.position]).await
    }

    /// Hands `job` out again after a wait that grows with its hand-outs; past the group's limit it goes to
    /// `media_failed` instead.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn failed(&self, job: &Job) -> Result<()> {
        let delay = topic_groups::retry_after(
            job.event.deliveries,
            MEDIA_RETRY_STEP_SECS,
            MEDIA_RETRY_MAX_SECS,
        );
        self.repository.nack(&[job.event.position], delay).await
    }
}
