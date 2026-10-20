//! An HLS stream rendition: ffmpeg writes the master playlist, one playlist per rung and their segments into a
//! scratch directory, and each file becomes an ordinary object under `<key>.renditions/hls-<rungs>/`, the master
//! last, so a reader that finds the master finds every file it names.

use std::path::{Path, PathBuf};

use tessari_s3_core::media::{Ladder, RENDITIONS};
use tessari_s3_infrastructure::media_video::Ffmpeg;
use tessari_s3_storage::encryption::Sealing;
use tessari_s3_storage::media::Rendition;
use tessari_s3_storage::objects::ObjectService;
use tessari_s3_types::{BucketName, ObjectKey};

use crate::media_work::landed;

/// The master playlist's file name, which every player opens first.
const MASTER: &str = "master.m3u8";

/// The `Content-Type` of a stream file, by its extension.
fn content_type(file: &Path) -> &'static str {
    match file.extension().and_then(|extension| extension.to_str()) {
        Some("m3u8") => "application/vnd.apple.mpegurl",
        Some("ts") => "video/mp2t",
        _ => "application/octet-stream",
    }
}

/// Every file under `directory`, one level of rung folders deep, the master last.
async fn files(directory: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    let mut folders = vec![directory.to_path_buf()];
    while let Some(folder) = folders.pop() {
        let mut entries = tokio::fs::read_dir(&folder).await?;
        while let Some(entry) = entries.next_entry().await? {
            if entry.file_type().await?.is_dir() {
                folders.push(entry.path());
            } else {
                found.push(entry.path());
            }
        }
    }
    found.sort_by_key(|file| (file.ends_with(MASTER), file.clone()));
    Ok(found)
}

/// Makes `ladder`'s stream of `input` in `directory` and writes it beside the original; `Err` carries why not.
pub(crate) async fn stream(
    ffmpeg: &Ffmpeg,
    objects: &ObjectService,
    (bucket, rendition): (&BucketName, &Rendition),
    (input, directory): (&Path, &Path),
    ladder: Ladder,
    sealing: Option<&Sealing>,
) -> Result<(), String> {
    let io = |error: std::io::Error| format!("a scratch file failed: {}", error.kind());
    tokio::fs::create_dir_all(directory).await.map_err(io)?;
    let audio = ffmpeg
        .has_audio(input)
        .await
        .map_err(|error| error.to_string())?;
    let rungs: Vec<u32> = ladder.rungs().collect();
    ffmpeg
        .stream(input, directory, &rungs, audio)
        .await
        .map_err(|error| error.to_string())?;
    let entry = rendition.output.file_name();
    let folder = format!(
        "{}{RENDITIONS}{}",
        rendition.key,
        entry.strip_suffix(MASTER).unwrap_or(&entry)
    );
    for file in files(directory).await.map_err(io)? {
        let relative = file
            .strip_prefix(directory)
            .map_err(|_| "a stream file is outside its folder".to_owned())?
            .to_string_lossy()
            .replace('\\', "/");
        let key = ObjectKey::new(&format!("{folder}{relative}"))
            .map_err(|_| "a stream file's key is longer than a key may be".to_owned())?;
        let length = tokio::fs::metadata(&file).await.map_err(io)?.len();
        let mut reader = tokio::fs::File::open(&file).await.map_err(io)?;
        let written = objects
            .put_reader(
                bucket,
                &key,
                &mut reader,
                length,
                content_type(&file),
                sealing,
            )
            .await
            .map_err(|error| error.to_string())?;
        landed(&written)?;
    }
    Ok(())
}
