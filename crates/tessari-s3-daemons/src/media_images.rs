//! An image worker: takes image rendition tasks from `media_images` under this node's consumer name, renders each from
//! the version it names and writes it beside the original under `<key>.renditions/`. A node runs as many as it is
//! configured to, each one task at a time; rendering is CPU work and runs off the async runtime.

use std::time::Duration;

use tessari_s3_constants::{
    DAEMON_RUN_TIMEOUT_SECS, MEDIA_BATCH, MEDIA_MAX_INPUT_BYTES, MEDIA_POLL_SECS,
};
use tessari_s3_core::media::{ImageFormat, Output, RENDITIONS};
use tessari_s3_infrastructure::media_image::{Encoding, render};
use tessari_s3_storage::media::{MediaService, Task, TaskQueue};
use tessari_s3_storage::objects::{Content, ObjectService, Written};
use tessari_s3_types::{BucketName, ObjectKey};

use crate::Error;
use crate::daemon::{Daemon, DaemonFuture, Report};

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
        let (Ok(bucket), Ok(key)) = (
            BucketName::new(&rendition.bucket),
            ObjectKey::new(&rendition.key),
        ) else {
            return Ok(());
        };
        let object = self
            .objects
            .stored(&bucket, &key, rendition.version_id.as_deref())
            .await
            .map_err(|error| error.to_string())?;
        let Some(object) = object.filter(|object| match &object.content {
            Content::Data(id) => *id == rendition.source,
            Content::Parts(multipart) => multipart.upload == rendition.source,
        }) else {
            return Ok(());
        };
        let original = self
            .objects
            .read_whole(&object, MEDIA_MAX_INPUT_BYTES)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "the original is larger than a media worker reads".to_owned())?;
        let encoding = match format {
            ImageFormat::Jpeg => Encoding::Jpeg,
            ImageFormat::Png => Encoding::Png,
            ImageFormat::Webp => Encoding::Webp,
            ImageFormat::Avif => Encoding::Avif,
        };
        // Bounded by the input guards before any decoding starts; it cannot be cancelled once running.
        let made = tokio::task::spawn_blocking(move || render(&original, &[(size, encoding)]))
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?
            .pop()
            .ok_or_else(|| "no rendition was made".to_owned())?;
        let target = ObjectKey::new(&format!(
            "{}{RENDITIONS}{}",
            rendition.key,
            rendition.output.file_name()
        ))
        .map_err(|_| "the rendition's key is longer than a key may be".to_owned())?;
        match self
            .objects
            .put_bytes(&bucket, &target, &made.bytes, format.content_type())
            .await
            .map_err(|error| error.to_string())?
        {
            Written::Committed { .. } | Written::NoSuchBucket => Ok(()),
            _ => Err("the rendition was not written".to_owned()),
        }
    }
}

fn failed(error: &tessari_s3_storage::Error) -> Error {
    Error::Run {
        daemon: "media-images",
        message: error.to_string(),
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
            let (mut made, mut retried) = (0_usize, 0_usize);
            for _ in 0..MEDIA_BATCH {
                let Some(task) = self
                    .media
                    .claim(TaskQueue::Images, &self.consumer, 1)
                    .await
                    .map_err(|e| failed(&e))?
                    .pop()
                else {
                    break;
                };
                match self.make(&task).await {
                    Ok(()) => {
                        made = made.saturating_add(1);
                        self.media.rendered(&task).await.map_err(|e| failed(&e))?;
                    }
                    Err(reason) => {
                        tracing::warn!(daemon = "media-images", task = %task.name, %reason, "a rendition was not made");
                        retried = retried.saturating_add(1);
                        self.media
                            .retry(&task, &reason)
                            .await
                            .map_err(|e| failed(&e))?;
                    }
                }
            }
            Ok(Report {
                message: format!("renditions made {made}, handed back {retried}"),
            })
        })
    }
}
