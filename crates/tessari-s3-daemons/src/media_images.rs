//! An image worker: takes image rendition tasks from `media_images` under this node's consumer name, renders each from
//! the version it names and writes it beside the original under `<key>.renditions/`. A node runs as many as it is
//! configured to, each one task at a time; rendering is CPU work and runs off the async runtime.

use std::time::Duration;

use tessari_s3_constants::{
    DAEMON_RUN_TIMEOUT_SECS, MEDIA_BATCH, MEDIA_MAX_INPUT_BYTES, MEDIA_POLL_SECS,
};
use tessari_s3_core::media::Output;
use tessari_s3_infrastructure::media_image::render;
use tessari_s3_storage::media::{MediaService, Task, TaskQueue};
use tessari_s3_storage::objects::ObjectService;

use crate::daemon::{Daemon, DaemonFuture, Report};
use crate::media_work::{encoding, landed, original, sealed, target, work};

/// Renders image tasks.
pub struct ImageWorker {
    media: MediaService,
    objects: ObjectService,
    consumer: String,
}

impl ImageWorker {
    /// A worker claiming from `media` as `consumer` and reading and writing objects through `objects`.
    #[must_use]
    pub const fn new(media: MediaService, objects: ObjectService, consumer: String) -> Self {
        Self {
            media,
            objects,
            consumer,
        }
    }

    /// Makes `task`'s rendition; `Err` carries why it was not made. A version gone or replaced since needs none.
    async fn make(&self, task: &Task) -> Result<(), String> {
        let rendition = &task.rendition;
        let Output::Image { size, format } = rendition.output else {
            return Err("not an image output".to_owned());
        };
        let Some((bucket, object)) = original(&self.objects, rendition).await? else {
            return Ok(());
        };
        let bytes = self
            .objects
            .read_whole(&object, MEDIA_MAX_INPUT_BYTES)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "the original is larger than a media worker reads".to_owned())?;
        let encoder = encoding(format);
        // Bounded by the input guards before any decoding starts; it cannot be cancelled once running.
        let made = tokio::task::spawn_blocking(move || render(&bytes, &[(size, encoder)]))
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?
            .pop()
            .ok_or_else(|| "no rendition was made".to_owned())?;
        let written = self
            .objects
            .put_bytes(
                &bucket,
                &target(rendition)?,
                &made.bytes,
                format.content_type(),
                sealed(&object)?.as_ref(),
            )
            .await
            .map_err(|error| error.to_string())?;
        landed(&written)
    }
}

impl Daemon for ImageWorker {
    fn name(&self) -> &'static str {
        "media-images"
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(MEDIA_POLL_SECS)
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(DAEMON_RUN_TIMEOUT_SECS)
    }

    fn run_once(&self) -> DaemonFuture<'_> {
        Box::pin(async move {
            let (made, retried) = work(
                "media-images",
                &self.media,
                TaskQueue::Images,
                &self.consumer,
                MEDIA_BATCH,
                |task| async move {
                    let outcome = self.make(&task).await;
                    (task, outcome)
                },
            )
            .await?;
            Ok(Report {
                message: format!("renditions made {made}, handed back {retried}"),
                idle: made == 0 && retried == 0,
            })
        })
    }
}
