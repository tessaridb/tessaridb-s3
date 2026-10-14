//! What the image and video workers share: the original a task names, if it is still that version; the rendition's
//! key beside it; and the loop that claims tasks one at a time and settles each by its outcome.

use std::future::Future;

use tessari_s3_core::media::{ImageFormat, RENDITIONS};
use tessari_s3_infrastructure::media_image::Encoding;
use tessari_s3_storage::media::{MediaService, Rendition, Task, TaskQueue};
use tessari_s3_storage::objects::{Content, ObjectService, StoredObject, Written};
use tessari_s3_types::{BucketName, ObjectKey};

use crate::Error;

/// The bucket, the original's key, and the original when it is still the version the task was planned from.
///
/// # Errors
/// Why it could not be read, as the task's error.
pub(crate) async fn original(
    objects: &ObjectService,
    rendition: &Rendition,
) -> Result<Option<(BucketName, StoredObject)>, String> {
    let (Ok(bucket), Ok(key)) = (
        BucketName::new(&rendition.bucket),
        ObjectKey::new(&rendition.key),
    ) else {
        return Ok(None);
    };
    let object = objects
        .stored(&bucket, &key, rendition.version_id.as_deref())
        .await
        .map_err(|error| error.to_string())?;
    Ok(object
        .filter(|object| match &object.content {
            Content::Data(id) => *id == rendition.source,
            Content::Parts(multipart) => multipart.upload == rendition.source,
        })
        .map(|object| (bucket, object)))
}

/// `<key>.renditions/<kind>-<size>.<ext>`.
///
/// # Errors
/// When the original's key leaves no room for the suffix.
pub(crate) fn target(rendition: &Rendition) -> Result<ObjectKey, String> {
    ObjectKey::new(&format!(
        "{}{RENDITIONS}{}",
        rendition.key,
        rendition.output.file_name()
    ))
    .map_err(|_| "the rendition's key is longer than a key may be".to_owned())
}

/// Whether a rendition write landed; a bucket deleted meanwhile needs none.
///
/// # Errors
/// Any other outcome, as the task's error.
pub(crate) fn landed(written: &Written) -> Result<(), String> {
    match written {
        Written::Committed { .. } | Written::NoSuchBucket => Ok(()),
        _ => Err("the rendition was not written".to_owned()),
    }
}

/// The image encoder for `format`.
pub(crate) const fn encoding(format: ImageFormat) -> Encoding {
    match format {
        ImageFormat::Jpeg => Encoding::Jpeg,
        ImageFormat::Png => Encoding::Png,
        ImageFormat::Webp => Encoding::Webp,
        ImageFormat::Avif => Encoding::Avif,
    }
}

/// Claims up to `limit` tasks of `queue` one at a time as `consumer` and settles each by what `make` answers: removed
/// when made, handed back with the reason when not. Answers how many were made and handed back.
///
/// # Errors
/// The metadata store's refusal or outage, naming `daemon`.
pub(crate) async fn work<'a, F, Fut>(
    daemon: &'static str,
    media: &'a MediaService,
    queue: TaskQueue,
    consumer: &'a str,
    limit: usize,
    make: F,
) -> Result<(usize, usize), Error>
where
    F: Fn(Task) -> Fut,
    Fut: Future<Output = (Task, Result<(), String>)>,
{
    let failed = |error: tessari_s3_storage::Error| Error::Run {
        daemon,
        message: error.to_string(),
    };
    let (mut made, mut retried) = (0_usize, 0_usize);
    for _ in 0..limit {
        let Some(task) = media.claim(queue, consumer, 1).await.map_err(failed)?.pop() else {
            break;
        };
        match make(task).await {
            (task, Ok(())) => {
                made = made.saturating_add(1);
                media.rendered(&task).await.map_err(failed)?;
            }
            (task, Err(reason)) => {
                tracing::warn!(daemon, task = %task.name, %reason, "a rendition was not made");
                retried = retried.saturating_add(1);
                media.retry(&task, &reason).await.map_err(failed)?;
            }
        }
    }
    Ok((made, retried))
}
