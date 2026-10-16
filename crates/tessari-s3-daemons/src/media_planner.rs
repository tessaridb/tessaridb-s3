//! The media planner: turns each created object its bucket's media rules take into rendition tasks — one per output,
//! queued for the encoder it needs, at most once per version — and then settles the object's message. Every node runs
//! one; the media group hands each object to one of them at a time.

use std::collections::BTreeSet;
use std::time::Duration;

use tessari_s3_constants::{DAEMON_RUN_TIMEOUT_SECS, MEDIA_BATCH, MEDIA_POLL_SECS};
use tessari_s3_core::media::{Output, matches};
use tessari_s3_storage::media::{Job, MediaService, Rendition};
use tessari_s3_storage::objects::{Content, ObjectService};
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

    /// Queues the renditions `job`'s object needs; answers how many were asked for. An object gone since needs none.
    async fn plan(&self, job: &Job) -> tessari_s3_storage::Result<usize> {
        let (Ok(bucket), Ok(key)) = (
            BucketName::new(&job.event.bucket),
            ObjectKey::new(&job.event.key),
        ) else {
            return Ok(0);
        };
        let Some(object) = self
            .objects
            .stored(&bucket, &key, job.event.version_id.as_deref())
            .await?
        else {
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
        let outputs: BTreeSet<Output> = job
            .rules
            .iter()
            .filter(|rule| matches(rule, content_type, &job.event.key))
            .flat_map(|rule| rule.outputs.iter().copied())
            .filter(|output| output.source() == family)
            .collect();
        let source = match &object.content {
            Content::Data(id) => *id,
            Content::Parts(multipart) => multipart.upload,
        };
        for output in &outputs {
            self.media
                .enqueue(&Rendition {
                    bucket: job.event.bucket.clone(),
                    key: job.event.key.clone(),
                    version_id: object.version.clone(),
                    source,
                    output: *output,
                })
                .await?;
        }
        Ok(outputs.len())
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
                match self.plan(&job).await {
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
            Ok(Report {
                message: format!("renditions queued {queued}, objects handed back {handed_back}"),
            })
        })
    }
}
