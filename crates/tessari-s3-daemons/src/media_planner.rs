//! The media planner: turns each created object its bucket's media rules take into rendition tasks — one per output,
//! queued for the encoder it needs, at most once per version — and then settles the object's message. Every node runs
//! one; the media group hands each object to one of them at a time. After the created objects, a pass plans one page
//! of a bucket's sweep, which reaches the objects written before a rule, skipping a rendition already made since.

use std::collections::BTreeSet;
use std::time::Duration;

use tessari_s3_constants::{
    DAEMON_RUN_TIMEOUT_SECS, MEDIA_BATCH, MEDIA_POLL_SECS, MEDIA_SWEEP_PAGE,
};
use tessari_s3_core::media::{Output, RENDITIONS, Rule, matches};
use tessari_s3_core::objects::listing::{Entry, ListSpec};
use tessari_s3_storage::media::{MediaService, Rendition};
use tessari_s3_storage::objects::{Content, Listed, ObjectService};
use tessari_s3_types::{BucketName, ObjectKey};

use crate::Error;
use crate::daemon::{Daemon, DaemonFuture, Report};

/// The content type an object without one is served as.
const UNTYPED: &str = "application/octet-stream";

/// Plans the renditions of the media group's objects.
pub struct MediaPlanner {
    media: MediaService,
    objects: ObjectService,
}

impl MediaPlanner {
    /// A planner taking jobs from `media` and reading objects through `objects`.
    #[must_use]
    pub const fn new(media: MediaService, objects: ObjectService) -> Self {
        Self { media, objects }
    }

    /// Queues the renditions `rules` ask of `key` (the version named, else the current one); answers how many were
    /// asked for. An object gone since needs none. With `skip_made`, an output whose rendition was written since the
    /// object is not queued again.
    async fn plan(
        &self,
        bucket_name: &str,
        key_name: &str,
        version_id: Option<&str>,
        rules: &[Rule],
        skip_made: bool,
    ) -> tessari_s3_storage::Result<usize> {
        let (Ok(bucket), Ok(key)) = (BucketName::new(bucket_name), ObjectKey::new(key_name)) else {
            return Ok(0);
        };
        let Some(object) = self.objects.stored(&bucket, &key, version_id).await? else {
            return Ok(0);
        };
        // A rendition would need the customer's key, which is never kept, and would be readable without it.
        if object.customer_seal().is_some() {
            return Ok(0);
        }
        let content_type = object
            .headers
            .get("content-type")
            .map_or(UNTYPED, String::as_str);
        // The rule matched the type family; an output of the other family is not made from it.
        let family = content_type
            .split('/')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        let outputs: BTreeSet<Output> = rules
            .iter()
            .filter(|rule| matches(rule, content_type, key_name))
            .flat_map(|rule| rule.outputs.iter().copied())
            .filter(|output| output.source() == family)
            .collect();
        let source = match &object.content {
            Content::Data(id) => *id,
            Content::Parts(multipart) => multipart.upload,
        };
        let mut queued = 0_usize;
        for output in &outputs {
            if skip_made
                && self
                    .made_since(&bucket, key_name, *output, &object.modified)
                    .await?
            {
                continue;
            }
            self.media
                .enqueue(&Rendition {
                    bucket: bucket_name.to_owned(),
                    key: key_name.to_owned(),
                    version_id: object.version.clone(),
                    source,
                    output: *output,
                })
                .await?;
            queued = queued.saturating_add(1);
        }
        Ok(queued)
    }

    /// Whether `output`'s rendition of `key` exists and was written no earlier than the object, `modified`.
    async fn made_since(
        &self,
        bucket: &BucketName,
        key: &str,
        output: Output,
        modified: &tessari_s3_types::Timestamp,
    ) -> tessari_s3_storage::Result<bool> {
        let Ok(rendition) = ObjectKey::new(&format!("{key}{RENDITIONS}{}", output.file_name()))
        else {
            return Ok(false);
        };
        Ok(self
            .objects
            .stored(bucket, &rendition, None)
            .await?
            .is_some_and(|made| made.modified >= *modified))
    }

    /// Plans one page of the next sweep with keys left; answers the renditions queued, or `None` with no sweep due.
    async fn sweep(&self) -> tessari_s3_storage::Result<Option<usize>> {
        let Some((mut sweep, rules)) = self.media.next_sweep().await? else {
            return Ok(None);
        };
        let page = match BucketName::new(&sweep.bucket) {
            Ok(bucket) if !rules.is_empty() => {
                let spec = ListSpec {
                    prefix: String::new(),
                    delimiter: None,
                    max_keys: MEDIA_SWEEP_PAGE,
                    start_after: (!sweep.after.is_empty()).then(|| sweep.after.clone()),
                    resume: None,
                };
                match self.objects.list(&bucket, spec).await? {
                    Listed::Page(page) => Some(page),
                    Listed::NoSuchBucket => None,
                }
            }
            // A bucket or rules gone since the sweep began: nothing is left to plan.
            _ => None,
        };
        let Some(page) = page else {
            self.media.advance_sweep(&sweep, true).await?;
            return Ok(Some(0));
        };
        let mut queued = 0_usize;
        for entry in &page.entries {
            let Entry::Object { key, .. } = entry else {
                continue;
            };
            if !key.contains(RENDITIONS) {
                queued =
                    queued.saturating_add(self.plan(&sweep.bucket, key, None, &rules, true).await?);
                sweep.listed = sweep.listed.saturating_add(1);
            }
            sweep.after.clone_from(key);
        }
        sweep.queued = sweep
            .queued
            .saturating_add(u64::try_from(queued).unwrap_or(u64::MAX));
        self.media.advance_sweep(&sweep, !page.truncated).await?;
        Ok(Some(queued))
    }
}

fn failed(error: &tessari_s3_storage::Error) -> Error {
    Error::Run {
        daemon: "media-planner",
        message: error.to_string(),
    }
}

impl Daemon for MediaPlanner {
    fn name(&self) -> &'static str {
        "media-planner"
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(MEDIA_POLL_SECS)
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(DAEMON_RUN_TIMEOUT_SECS)
    }

    fn run_once(&self) -> DaemonFuture<'_> {
        Box::pin(async move {
            let (mut queued, mut handed_back) = (0_usize, 0_usize);
            for job in self.media.next(MEDIA_BATCH).await.map_err(|e| failed(&e))? {
                let event = &job.event;
                match self
                    .plan(
                        &event.bucket,
                        &event.key,
                        event.version_id.as_deref(),
                        &job.rules,
                        false,
                    )
                    .await
                {
                    Ok(count) => {
                        queued = queued.saturating_add(count);
                        self.media.done(&job).await.map_err(|e| failed(&e))?;
                    }
                    Err(error) => {
                        tracing::warn!(daemon = "media-planner", %error, "an object's renditions were not queued");
                        handed_back = handed_back.saturating_add(1);
                        self.media.failed(&job).await.map_err(|e| failed(&e))?;
                    }
                }
            }
            let swept = self.sweep().await.map_err(|e| failed(&e))?;
            Ok(Report {
                message: format!(
                    "renditions queued {queued}, objects handed back {handed_back}, swept {}",
                    swept.unwrap_or(0)
                ),
                idle: queued == 0 && handed_back == 0 && swept.is_none(),
            })
        })
    }
}
