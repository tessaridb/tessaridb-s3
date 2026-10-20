//! An HLS output against a real node and the operator's ffmpeg (G023 C5): one rendition per rung under a master
//! playlist, every file an ordinary object under the original's `.renditions/` folder, and ffprobe reads the master.

use crate::media::{ffmpeg, probe};
use crate::{IGNORED, call, call_with, fresh};

/// A seven-second 1280×720 clip, with a tone when `audio`.
async fn clip(audio: bool) -> Vec<u8> {
    let file = std::env::temp_dir().join(format!(
        "tessaridb-s3-clip-{}.mp4",
        uuid::Uuid::new_v4().simple()
    ));
    let mut command = tokio::process::Command::new(ffmpeg());
    command.args([
        "-v",
        "error",
        "-y",
        "-f",
        "lavfi",
        "-i",
        "testsrc=size=1280x720:rate=10:duration=7",
    ]);
    if audio {
        command.args([
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=7",
            "-c:a",
            "aac",
            "-shortest",
        ]);
    }
    let made = command
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p"])
        .arg(&file)
        .status()
        .await
        .expect("ffmpeg ran");
    assert!(made.success(), "the clip was made");
    let bytes = tokio::fs::read(&file).await.expect("clip read");
    tokio::fs::remove_file(&file).await.expect("clip removed");
    bytes
}

#[tokio::test]
#[ignore = "needs a TessariDB node and ffmpeg: set TESSARIDB_S3_TEST_META, _USER, _PASSWORD and _FFMPEG"]
async fn a_video_gets_an_hls_ladder_under_a_master_that_ffprobe_reads() {
    use tessari_s3_daemons::{Daemon, MediaPlanner, VideoWorker};
    use tessari_s3_infrastructure::media_video::Ffmpeg;
    let _why = IGNORED;
    let (state, planter) = fresh().await;
    assert_eq!(call(&state, "PUT", "/films", vec![], b"").await.status, 200);
    planter
        .run(
            &format!(
                "UPDATE {}.buckets:'films' SET media = [{{ id: 'hls', content_types: ['video/*'], prefix: '', \
                 outputs: ['video/360p+720p/hls'] }}];",
                planter.global_database()
            ),
            Vec::new(),
        )
        .await
        .expect("media rules set");
    for (key, audio) in [("loud.mp4", true), ("silent.mp4", false)] {
        let put = call_with(
            &state,
            "PUT",
            &format!("/films/{key}"),
            vec![],
            vec![("content-type", "video/mp4")],
            &clip(audio).await,
        )
        .await;
        assert_eq!(put.status, 200, "{key}");
    }
    let (media, objects) = (state.storage().media(), state.storage().objects().clone());
    media.define().await.expect("group defined");
    MediaPlanner::new(media.clone(), objects.clone())
        .run_once()
        .await
        .expect("planned");
    let worker = VideoWorker::new(
        media,
        objects,
        "node-a".to_owned(),
        Ffmpeg::new(ffmpeg()),
        std::env::temp_dir(),
    );
    for _ in 0..2 {
        worker.run_once().await.expect("a video run");
    }
    for (key, audio) in [("loud.mp4", true), ("silent.mp4", false)] {
        let at = format!("/films/{key}.renditions/hls-360p-720p");
        let fetched = |path: String, content_type: &'static str| {
            let state = &state;
            async move {
                let seen = call(state, "GET", &path, vec![], b"").await;
                assert_eq!(seen.status, 200, "{path}");
                assert_eq!(
                    seen.headers
                        .get("content-type")
                        .and_then(|value| value.to_str().ok()),
                    Some(content_type),
                    "{path}"
                );
                seen.bytes
            }
        };
        let master = fetched(format!("{at}/master.m3u8"), "application/vnd.apple.mpegurl").await;
        let master = String::from_utf8(master).expect("a text playlist");
        for expected in [
            "RESOLUTION=640x360",
            "RESOLUTION=1280x720",
            "360p/index.m3u8",
            "720p/index.m3u8",
            "avc1.",
        ] {
            assert!(master.contains(expected), "{key}: {expected} in {master}");
        }
        assert_eq!(
            master.contains("mp4a."),
            audio,
            "{key}: audio named only when there is audio"
        );
        // Mirrored to a folder so ffprobe follows the relative names the way a player does.
        let folder = std::env::temp_dir().join(format!(
            "tessaridb-s3-hls-{}",
            uuid::Uuid::new_v4().simple()
        ));
        for rung in ["360p", "720p"] {
            tokio::fs::create_dir_all(folder.join(rung))
                .await
                .expect("folder made");
            let index = fetched(
                format!("{at}/{rung}/index.m3u8"),
                "application/vnd.apple.mpegurl",
            )
            .await;
            let text = String::from_utf8(index.clone()).expect("a text playlist");
            assert!(
                text.contains("#EXT-X-PLAYLIST-TYPE:VOD") && text.contains("#EXT-X-ENDLIST"),
                "{text}"
            );
            tokio::fs::write(folder.join(rung).join("index.m3u8"), &index)
                .await
                .expect("written");
            for segment in ["seg-00000.ts", "seg-00001.ts"] {
                assert!(text.contains(segment), "{key} {rung}: {segment} in {text}");
                let bytes = fetched(format!("{at}/{rung}/{segment}"), "video/mp2t").await;
                if rung == "360p" && segment == "seg-00000.ts" {
                    // A transport stream lists its video under the program too: the first line is the stream.
                    let seen = probe(&bytes, "v").await;
                    assert_eq!(seen.lines().next(), Some("h264,640,360,yuv420p"));
                }
                tokio::fs::write(folder.join(rung).join(segment), &bytes)
                    .await
                    .expect("written");
            }
        }
        tokio::fs::write(folder.join("master.m3u8"), master.as_bytes())
            .await
            .expect("written");
        let programs = tokio::process::Command::new(ffmpeg().with_file_name("ffprobe"))
            .args([
                "-v",
                "error",
                "-show_entries",
                "program=program_id",
                "-of",
                "default=nw=1",
            ])
            .arg(folder.join("master.m3u8"))
            .output()
            .await
            .expect("ffprobe ran");
        assert!(programs.status.success(), "{key}: ffprobe read the master");
        assert_eq!(
            String::from_utf8_lossy(&programs.stdout)
                .lines()
                .filter(|line| line.starts_with("program_id="))
                .count(),
            2,
            "{key}: one program per rung"
        );
        tokio::fs::remove_dir_all(&folder)
            .await
            .expect("folder removed");
    }
}
