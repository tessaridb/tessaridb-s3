//! A video worker, run on a node with ffmpeg configured: takes video and poster tasks from `media_videos` under this
//! node's consumer name, copies the version a task names to a file, has ffmpeg make the video or a frame — a poster's
//! frame is then fitted and encoded as an image — and writes the result beside the original under `<key>.renditions/`.
//! One task per run; the files are removed on every path.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tessari_s3_constants::{
    DAEMON_RUN_TIMEOUT_SECS, MEDIA_FFMPEG_TIMEOUT_SECS, MEDIA_MAX_VIDEO_INPUT_BYTES,
    MEDIA_POLL_SECS,
};
use tessari_s3_core::media::{Output, VideoFormat};
use tessari_s3_infrastructure::media_image::render;
use tessari_s3_infrastructure::media_video::{Ffmpeg, VideoEncoding};
use tessari_s3_storage::media::{MediaService, Task, TaskQueue};
use tessari_s3_storage::objects::{ObjectService, StoredObject};
use tessari_s3_types::BucketName;

use crate::daemon::{Daemon, DaemonFuture, Report};
use crate::media_work::{encoding, landed, original, sealed, target, work};

/// Renders video and poster tasks through ffmpeg.
pub struct VideoWorker {
    media: MediaService,
    objects: ObjectService,
    consumer: String,
    ffmpeg: Ffmpeg,
    scratch: PathBuf,
}

impl VideoWorker {
    /// A worker claiming from `media` as `consumer`, reading and writing objects through `objects`, running `ffmpeg`
    /// on files in `scratch`.
    #[must_use]
    pub const fn new(
        media: MediaService,
        objects: ObjectService,
        consumer: String,
        ffmpeg: Ffmpeg,
        scratch: PathBuf,
    ) -> Self {
        Self {
            media,
            objects,
            consumer,
            ffmpeg,
            scratch,
        }
    }

    /// Makes `task`'s rendition; `Err` carries why it was not made. A version gone or replaced since needs none.
    async fn make(&self, task: &Task) -> Result<(), String> {
        let Some((bucket, object)) = original(&self.objects, &task.rendition).await? else {
            return Ok(());
        };
        let named = uuid::Uuid::new_v4().simple();
        let input = self.scratch.join(format!("tessaridb-s3-media-{named}.in"));
        let output = self.scratch.join(format!("tessaridb-s3-media-{named}.out"));
        let made = self
            .make_from(task, &bucket, &object, &input, &output)
            .await;
        for file in [&input, &output] {
            if let Err(error) = tokio::fs::remove_file(file).await
                && error.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!(daemon = "media-videos", %error, "a scratch file was left behind");
            }
        }
        made
    }

    async fn make_from(
        &self,
        task: &Task,
        bucket: &BucketName,
        object: &StoredObject,
        input: &Path,
        output: &Path,
    ) -> Result<(), String> {
        let io = |error: std::io::Error| format!("a scratch file failed: {}", error.kind());
        let mut file = tokio::fs::File::create(input).await.map_err(io)?;
        if !self
            .objects
            .copy_to(object, MEDIA_MAX_VIDEO_INPUT_BYTES, &mut file)
            .await
            .map_err(|error| error.to_string())?
        {
            return Err("the original is larger than a video worker reads".to_owned());
        }
        drop(file);
        let rendition = &task.rendition;
        let written = match rendition.output {
            Output::Video { size, format } => {
                let encoder = match format {
                    VideoFormat::Mp4 => VideoEncoding::Mp4,
                    VideoFormat::Webm => VideoEncoding::Webm,
                };
                self.ffmpeg
                    .video(input, output, size, encoder)
                    .await
                    .map_err(|error| error.to_string())?;
                let length = tokio::fs::metadata(output).await.map_err(io)?.len();
                let mut made = tokio::fs::File::open(output).await.map_err(io)?;
                self.objects
                    .put_reader(
                        bucket,
                        &target(rendition)?,
                        &mut made,
                        length,
                        format.content_type(),
                        sealed(object),
                    )
                    .await
            }
            Output::Poster { size, format } => {
                self.ffmpeg
                    .frame(input, output)
                    .await
                    .map_err(|error| error.to_string())?;
                let frame = tokio::fs::read(output).await.map_err(io)?;
                let encoder = encoding(format);
                let made = tokio::task::spawn_blocking(move || render(&frame, &[(size, encoder)]))
                    .await
                    .map_err(|error| error.to_string())?
                    .map_err(|error| error.to_string())?
                    .pop()
                    .ok_or_else(|| "no poster was made".to_owned())?;
                self.objects
                    .put_bytes(
                        bucket,
                        &target(rendition)?,
                        &made.bytes,
                        format.content_type(),
                        sealed(object),
                    )
                    .await
            }
            Output::Image { .. } => return Err("not a video output".to_owned()),
        };
        landed(&written.map_err(|error| error.to_string())?)
    }
}

impl Daemon for VideoWorker {
    fn name(&self) -> &'static str {
        "media-videos"
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(MEDIA_POLL_SECS)
    }

    /// One ffmpeg run's limit plus the copies around it.
    fn timeout(&self) -> Duration {
        Duration::from_secs(MEDIA_FFMPEG_TIMEOUT_SECS.saturating_add(DAEMON_RUN_TIMEOUT_SECS))
    }

    fn run_once(&self) -> DaemonFuture<'_> {
        Box::pin(async move {
            let (made, retried) = work(
                "media-videos",
                &self.media,
                TaskQueue::Videos,
                &self.consumer,
                1,
                |task| async move {
                    let outcome = self.make(&task).await;
                    (task, outcome)
                },
            )
            .await?;
            Ok(Report {
                message: format!("renditions made {made}, handed back {retried}"),
            })
        })
    }
}
