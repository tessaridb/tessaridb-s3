//! Video renditions and poster frames through the operator's ffmpeg, run as a separate process: no shell, every codec,
//! pixel format and container stated, the input and output files named by the caller, the process killed when its
//! wall-clock limit passes or its future is dropped. This server links no video codec.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tessari_s3_constants::MEDIA_FFMPEG_TIMEOUT_SECS;
use tokio::process::Command;

/// A video container and the codecs it is written with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoEncoding {
    /// MP4: H.264 High, 8-bit 4:2:0, AAC, the index at the front so it plays while downloading.
    Mp4,
    /// WebM: VP9 at constant quality, 8-bit 4:2:0, Opus.
    Webm,
}

/// Why ffmpeg made nothing.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VideoError {
    /// The configured ffmpeg could not be started.
    #[error("the configured ffmpeg could not be started")]
    Start,
    /// It ran past its wall-clock limit and was stopped.
    #[error("ffmpeg ran past its time limit")]
    TimedOut,
    /// It exited without success: the original is not a video it reads, or has no frame.
    #[error("ffmpeg could not make the rendition from the original")]
    Failed,
}

/// The operator's ffmpeg.
#[derive(Debug, Clone)]
pub struct Ffmpeg {
    binary: PathBuf,
}

impl Ffmpeg {
    /// The ffmpeg at `binary`.
    #[must_use]
    pub const fn new(binary: PathBuf) -> Self {
        Self { binary }
    }

    /// `input` rewritten as `encoding` into `output`, its shorter side `shorter_side` pixels at most — never enlarged,
    /// both sides even — with its first audio track if it has one.
    ///
    /// # Errors
    /// [`VideoError`] when ffmpeg cannot start, runs too long, or fails.
    pub async fn video(
        &self,
        input: &Path,
        output: &Path,
        shorter_side: u32,
        encoding: VideoEncoding,
    ) -> Result<(), VideoError> {
        let side = shorter_side;
        // Landscape fixes the height, portrait the width; -2 keeps the aspect with an even size.
        let scale = format!(
            "scale='if(gt(iw,ih),-2,trunc(min({side},iw)/2)*2)':'if(gt(iw,ih),trunc(min({side},ih)/2)*2,-2)'"
        );
        let mut args = vec!["-map", "0:v:0", "-map", "0:a:0?", "-vf", &scale];
        args.extend_from_slice(match encoding {
            VideoEncoding::Mp4 => &[
                "-c:v",
                "libx264",
                "-profile:v",
                "high",
                "-preset",
                "medium",
                "-crf",
                "23",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-b:a",
                "128k",
                "-movflags",
                "+faststart",
                "-f",
                "mp4",
            ],
            VideoEncoding::Webm => &[
                "-c:v",
                "libvpx-vp9",
                "-crf",
                "32",
                "-b:v",
                "0",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "libopus",
                "-b:a",
                "96k",
                "-f",
                "webm",
            ],
        });
        self.run(input, &args, output).await
    }

    /// A representative frame of `input` at its own size, as PNG into `output`.
    ///
    /// # Errors
    /// [`VideoError`] when ffmpeg cannot start, runs too long, or fails.
    pub async fn frame(&self, input: &Path, output: &Path) -> Result<(), VideoError> {
        // `thumbnail` picks the most typical frame of the first hundred rather than a black first one.
        let args = [
            "-map",
            "0:v:0",
            "-vf",
            "thumbnail",
            "-frames:v",
            "1",
            "-c:v",
            "png",
            "-f",
            "image2",
        ];
        self.run(input, &args, output).await
    }

    async fn run(&self, input: &Path, args: &[&str], output: &Path) -> Result<(), VideoError> {
        let mut all: Vec<OsString> = ["-hide_banner", "-nostdin", "-loglevel", "error", "-y", "-i"]
            .iter()
            .map(OsString::from)
            .collect();
        all.push(input.as_os_str().to_owned());
        all.extend(args.iter().map(OsString::from));
        all.push(output.as_os_str().to_owned());
        let mut child = Command::new(&self.binary)
            .args(all)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| VideoError::Start)?;
        match tokio::time::timeout(Duration::from_secs(MEDIA_FFMPEG_TIMEOUT_SECS), child.wait())
            .await
        {
            Ok(Ok(status)) if status.success() => Ok(()),
            Ok(_) => Err(VideoError::Failed),
            // Dropping the child on return kills it.
            Err(_) => Err(VideoError::TimedOut),
        }
    }
}
