//! Video renditions and poster frames through the operator's ffmpeg, run as a separate process: no shell, every codec,
//! pixel format and container stated, the input and output files named by the caller, the process killed when its
//! wall-clock limit passes or its future is dropped. This server links no video codec. An HLS stream is written into a
//! directory the caller names and then reads back file by file.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tessari_s3_constants::{MEDIA_FFMPEG_INPUT_FORMATS, MEDIA_FFMPEG_TIMEOUT_SECS};
use tokio::process::Command;

/// A video container and the codecs it is written with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoEncoding {
    /// MP4: H.264 High, 8-bit 4:2:0, AAC, the index at the front so it plays while downloading.
    Mp4,
    /// WebM: VP9 at constant quality, 8-bit 4:2:0, Opus.
    Webm,
}

/// The most kilobits a second a stream rung of `side` lines may take: the common streaming ladder's figures.
const fn rung_cap_kbps(side: u32) -> u32 {
    match side {
        0..=360 => 1_000,
        361..=480 => 1_500,
        481..=720 => 3_000,
        721..=1080 => 6_000,
        1081..=1440 => 10_000,
        _ => 16_000,
    }
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

    /// Whether `input` has an audio track ffmpeg decodes: one frame of it is decoded and thrown away.
    ///
    /// # Errors
    /// [`VideoError`] when ffmpeg cannot start or runs too long.
    pub async fn has_audio(&self, input: &Path) -> Result<bool, VideoError> {
        let args = ["-map", "0:a:0", "-frames:a", "1", "-f", "null"];
        match self.run(input, &args, Path::new("-")).await {
            Ok(()) => Ok(true),
            Err(VideoError::Failed) => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// `input` as an HLS stream in `directory`: per rung of `rungs` (shorter sides, ascending) an H.264 rendition, never
    /// enlarged, in six-second segments starting on keyframes under `<rung>p/index.m3u8`, and `master.m3u8` naming
    /// them with their bandwidth, resolution and codecs; AAC audio when `audio`.
    ///
    /// # Errors
    /// [`VideoError`] when ffmpeg cannot start, runs too long, or fails.
    pub async fn stream(
        &self,
        input: &Path,
        directory: &Path,
        rungs: &[u32],
        audio: bool,
    ) -> Result<(), VideoError> {
        let labels: Vec<String> = (0..rungs.len())
            .map(|index| format!("[s{index}]"))
            .collect();
        let mut graph = format!("[0:v:0]split={}{}", rungs.len(), labels.concat());
        let mut args: Vec<String> = Vec::new();
        let mut variants = Vec::new();
        for (index, side) in rungs.iter().enumerate() {
            graph.push_str(&format!(
                ";[s{index}]scale='if(gt(iw,ih),-2,trunc(min({side},iw)/2)*2)':'if(gt(iw,ih),trunc(min({side},ih)/2)*2,-2)'[o{index}]"
            ));
            args.extend(["-map".to_owned(), format!("[o{index}]")]);
            if audio {
                args.extend(["-map".to_owned(), "0:a:0".to_owned()]);
            }
            // CRF capped per rung, so a player can trust the master's bandwidth to pick one.
            let cap = rung_cap_kbps(*side);
            args.extend([
                format!("-maxrate:v:{index}"),
                format!("{cap}k"),
                format!("-bufsize:v:{index}"),
                format!("{}k", cap.saturating_mul(2)),
            ]);
            variants.push(if audio {
                format!("v:{index},a:{index},name:{side}p")
            } else {
                format!("v:{index},name:{side}p")
            });
        }
        let segments = directory.join("%v").join("seg-%05d.ts");
        let mut all = vec!["-filter_complex".to_owned(), graph];
        all.extend(args);
        all.extend(
            [
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
                // A keyframe every six seconds and none placed by scene cuts: every segment starts on one.
                "-force_key_frames",
                "expr:gte(t,n_forced*6)",
                "-sc_threshold",
                "0",
                "-c:a",
                "aac",
                "-b:a",
                "128k",
                "-f",
                "hls",
                "-hls_time",
                "6",
                "-hls_playlist_type",
                "vod",
                "-hls_flags",
                "independent_segments",
                "-hls_segment_type",
                "mpegts",
                "-master_pl_name",
                "master.m3u8",
            ]
            .map(str::to_owned),
        );
        all.extend([
            "-hls_segment_filename".to_owned(),
            segments.to_string_lossy().into_owned(),
            "-var_stream_map".to_owned(),
            variants.join(" "),
        ]);
        let borrowed: Vec<&str> = all.iter().map(String::as_str).collect();
        self.run(input, &borrowed, &directory.join("%v").join("index.m3u8"))
            .await
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
        let mut child = Command::new(&self.binary)
            .args(command_line(input, args, output))
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

/// The arguments one ffmpeg run is given: `input` read with `args` applied, written to `output`. The original is read
/// through the file protocol only and by a listed container's demuxer only — a playlist or a concat list inside an
/// uploaded file would otherwise make ffmpeg open other files or the network, whatever the operator's ffmpeg version.
fn command_line(input: &Path, args: &[&str], output: &Path) -> Vec<OsString> {
    let mut all: Vec<OsString> = [
        "-hide_banner",
        "-nostdin",
        "-loglevel",
        "error",
        "-y",
        "-protocol_whitelist",
        "file",
        "-format_whitelist",
        MEDIA_FFMPEG_INPUT_FORMATS,
        "-i",
    ]
    .iter()
    .map(OsString::from)
    .collect();
    all.push(input.as_os_str().to_owned());
    all.extend(args.iter().map(OsString::from));
    all.push(output.as_os_str().to_owned());
    all
}

#[cfg(test)]
#[path = "media_video_tests.rs"]
mod tests;
