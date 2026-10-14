//! Media processing against a real node (G004): a bucket's media rules, and the changes a media-only bucket records so
//! its media worker can find them.

use tessari_s3_infrastructure::tessaridb::{Answer, MetaPool, Number, Value};

use crate::notifications::recorded;
use crate::{IGNORED, call, call_with, fresh};

/// How many messages the media group holds in flight, as the topic reports it.
async fn in_flight(planter: &MetaPool) -> i64 {
    let answers = planter
        .run("INFO FOR TOPIC object_events;", Vec::new())
        .await
        .expect("topic described");
    let Some(Answer::Value { value, .. }) = answers.into_iter().next() else {
        panic!("INFO FOR TOPIC answered no value");
    };
    let field = |value: Option<Value>, name: &str| match value {
        Some(Value::Object(mut fields)) => fields.remove(name),
        _ => None,
    };
    match field(field(field(Some(value), "groups"), "media"), "in_flight") {
        Some(Value::Number(Number::Integer(count))) => count,
        other => panic!("no media group in the topic's description: {other:?}"),
    }
}

/// Gives `bucket` media rules as the console API stores them.
async fn with_media(planter: &MetaPool, bucket: &str) {
    planter
        .run(
            "UPDATE buckets:$name SET media = [{ id: 'photos', content_types: ['image/*'], prefix: '', \
             outputs: ['image/320/jpeg'] }];",
            vec![("name".to_owned(), Value::String(bucket.to_owned()))],
        )
        .await
        .expect("media rules set");
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_bucket_with_media_rules_records_its_changes_without_a_notification_configuration() {
    let _why = IGNORED;
    let (state, planter) = fresh().await;
    for name in ["pictures", "plain"] {
        assert_eq!(
            call(&state, "PUT", &format!("/{name}"), vec![], b"")
                .await
                .status,
            200
        );
    }
    with_media(&planter, "pictures").await;
    call(&state, "PUT", "/pictures/a.jpg", vec![], b"jpeg").await;
    call(&state, "PUT", "/plain/b.jpg", vec![], b"jpeg").await;
    assert_eq!(
        recorded(&planter, "a.jpg").await,
        vec!["s3:ObjectCreated:Put".to_owned()]
    );
    assert!(
        recorded(&planter, "b.jpg").await.is_empty(),
        "a bucket with neither rules nor a configuration records nothing"
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn the_media_group_hands_out_only_objects_created_where_a_rule_takes_them() {
    let _why = IGNORED;
    let (state, planter) = fresh().await;
    for name in ["pictures", "plain"] {
        assert_eq!(
            call(&state, "PUT", &format!("/{name}"), vec![], b"")
                .await
                .status,
            200
        );
    }
    with_media(&planter, "pictures").await;
    call(&state, "PUT", "/pictures/a.jpg", vec![], b"jpeg").await;
    call(
        &state,
        "PUT",
        "/pictures/a.jpg.renditions/thumb.jpg",
        vec![],
        b"jpeg",
    )
    .await;
    call(&state, "DELETE", "/pictures/a.jpg", vec![], b"").await;
    call(&state, "PUT", "/plain/b.jpg", vec![], b"jpeg").await;
    let media = state.storage().media();
    media.define().await.expect("group defined");
    let jobs = media.next(16).await.expect("jobs read");
    let keys: Vec<(&str, &str)> = jobs
        .iter()
        .map(|job| (job.event.bucket.as_str(), job.event.key.as_str()))
        .collect();
    assert_eq!(
        keys,
        vec![("pictures", "a.jpg")],
        "a removal, a rendition and a bucket without rules make no job"
    );
    assert_eq!(
        in_flight(&planter).await,
        1,
        "what no rule takes is settled at once; only the job is held"
    );
    for job in &jobs {
        media.done(job).await.expect("settled");
    }
    assert_eq!(in_flight(&planter).await, 0, "a done job is settled");
    assert!(
        media.next(16).await.expect("jobs read").is_empty(),
        "what was settled is not handed out again"
    );
}

/// The `field` values of every record in `queue`, as the planter reads them.
async fn in_queue(planter: &MetaPool, queue: &str, field: &str) -> Vec<Value> {
    let answers = planter
        .run(&format!("SELECT {field} FROM {queue};"), Vec::new())
        .await
        .expect("queue read");
    match answers.into_iter().next() {
        Some(Answer::Records { records, .. }) => records
            .into_iter()
            .map(|(_, record)| match record {
                Value::Object(mut fields) => fields.remove(field).unwrap_or(Value::None),
                other => panic!("not a record: {other:?}"),
            })
            .collect(),
        _ => Vec::new(),
    }
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn renditions_wait_once_in_their_encoder_s_queue_and_are_held_retried_and_given_up() {
    use tessari_s3_core::media::Output;
    use tessari_s3_storage::media::{Rendition, TaskQueue};
    let _why = IGNORED;
    let (state, planter) = fresh().await;
    let media = state.storage().media();
    let rendition = |output: &str| Rendition {
        bucket: "pictures".to_owned(),
        key: "a.mov".to_owned(),
        version_id: None,
        source: [7; 16],
        output: Output::parse(output).expect("an output"),
    };
    let (thumb, poster, spent) = (
        rendition("image/320/jpeg"),
        rendition("poster/640/webp"),
        rendition("image/640/png"),
    );
    for queued in [&thumb, &thumb, &poster] {
        media.enqueue(queued).await.expect("queued");
    }
    // Each output waits once, in the queue of the encoder it needs.
    let images = media
        .claim(TaskQueue::Images, "node-a", 10)
        .await
        .expect("images claimed");
    let videos = media
        .claim(TaskQueue::Videos, "node-a", 10)
        .await
        .expect("videos claimed");
    let made = |tasks: &[tessari_s3_storage::media::Task]| {
        tasks
            .iter()
            .map(|task| task.rendition.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(made(&images), vec![thumb.clone()]);
    assert_eq!(made(&videos), vec![poster.clone()]);
    assert_eq!(images[0].attempts, 1);
    assert_eq!(images[0].consumer.as_deref(), Some("node-a"));
    assert!(
        media
            .claim(TaskQueue::Images, "node-b", 10)
            .await
            .expect("claimed")
            .is_empty(),
        "a held task goes to one consumer"
    );
    // A failure is recorded and the task waits before anyone takes it again.
    media
        .retry(&images[0], "decoder refused")
        .await
        .expect("retried");
    assert_eq!(
        in_queue(&planter, "media_images", "error").await,
        vec![Value::String("decoder refused".to_owned())]
    );
    assert!(
        media
            .claim(TaskQueue::Images, "node-b", 10)
            .await
            .expect("claimed")
            .is_empty(),
        "a retried task waits"
    );
    // A written rendition leaves its queue.
    media.rendered(&videos[0]).await.expect("settled");
    assert!(
        in_queue(&planter, "media_videos", "output")
            .await
            .is_empty()
    );
    // Five hand-outs spend a task, and it stays visible as given up.
    media.enqueue(&spent).await.expect("queued");
    let named = "07070707-0707-0707-0707-070707070707-image-640-png";
    for _ in 0..5 {
        planter
            .run(
                "CLAIM media_images:$name; RELEASE media_images:$name;",
                vec![("name".to_owned(), Value::String(named.to_owned()))],
            )
            .await
            .expect("handed out");
    }
    let given_up = media.given_up(TaskQueue::Images, 10).await.expect("listed");
    assert_eq!(made(&given_up), vec![spent]);
    assert_eq!(given_up[0].attempts, 5);
}

/// A `width` × `height` PNG.
fn png(width: u32, height: u32) -> Vec<u8> {
    let image = image::RgbImage::from_pixel(width, height, image::Rgb([40, 120, 200]));
    let mut out = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut out, image::ImageFormat::Png)
        .expect("png written");
    out.into_inner()
}

/// The size of the image at `path`, which must be served as `content_type` and be written in `format`.
async fn image_at(
    state: &tessari_s3_api::ApiState,
    path: &str,
    content_type: &str,
    format: image::ImageFormat,
) -> (u32, u32) {
    let seen = call(state, "GET", path, vec![], b"").await;
    assert_eq!(seen.status, 200, "{path}");
    assert_eq!(
        seen.headers
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some(content_type),
        "{path}"
    );
    assert_eq!(
        image::guess_format(&seen.bytes).ok(),
        Some(format),
        "{path}"
    );
    let image = image::load_from_memory(&seen.bytes).expect("an image");
    (image.width(), image.height())
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn an_uploaded_image_gets_its_renditions_from_two_nodes_and_a_rendition_gets_none() {
    use tessari_s3_daemons::{Daemon, ImageWorker, MediaPlanner};
    let _why = IGNORED;
    let (state, planter) = fresh().await;
    assert_eq!(
        call(&state, "PUT", "/pictures", vec![], b"").await.status,
        200
    );
    planter
        .run(
            "UPDATE buckets:'pictures' SET media = [{ id: 'photos', content_types: ['image/*'], \
             prefix: 'photos/', outputs: ['image/320/jpeg', 'image/1920/webp', 'image/150/png'] }];",
            Vec::new(),
        )
        .await
        .expect("media rules set");
    let put = call_with(
        &state,
        "PUT",
        "/pictures/photos/a.png",
        vec![],
        vec![("content-type", "image/png")],
        &png(2000, 1000),
    )
    .await;
    assert_eq!(put.status, 200);
    let (media, objects) = (state.storage().media(), state.storage().objects().clone());
    media.define().await.expect("group defined");
    let planner = MediaPlanner::new(media.clone(), objects.clone());
    planner.run_once().await.expect("planned");
    // Two nodes' workers share the queue; each task is made once, by whichever holds it.
    let node_a = ImageWorker::new(media.clone(), objects.clone(), "node-a".to_owned());
    let node_b = ImageWorker::new(media.clone(), objects.clone(), "node-b".to_owned());
    let (ran_a, ran_b) = tokio::join!(node_a.run_once(), node_b.run_once());
    ran_a.expect("node a ran");
    ran_b.expect("node b ran");
    let at = "/pictures/photos/a.png.renditions";
    assert_eq!(
        image_at(
            &state,
            &format!("{at}/image-320.jpg"),
            "image/jpeg",
            image::ImageFormat::Jpeg
        )
        .await,
        (320, 160)
    );
    assert_eq!(
        image_at(
            &state,
            &format!("{at}/image-1920.webp"),
            "image/webp",
            image::ImageFormat::WebP
        )
        .await,
        (1920, 960)
    );
    assert_eq!(
        image_at(
            &state,
            &format!("{at}/image-150.png"),
            "image/png",
            image::ImageFormat::Png
        )
        .await,
        (150, 75)
    );
    assert!(
        in_queue(&planter, "media_images", "output")
            .await
            .is_empty(),
        "every task is settled"
    );
    // The renditions' own creation is seen by the planner and plans nothing.
    planner.run_once().await.expect("planned again");
    assert!(
        in_queue(&planter, "media_images", "output")
            .await
            .is_empty()
    );
    assert_eq!(in_flight(&planter).await, 0, "every object is settled");
}

/// The ffmpeg the video test runs, named by `TESSARIDB_S3_TEST_FFMPEG`; its ffprobe sits beside it.
fn ffmpeg() -> std::path::PathBuf {
    std::env::var_os("TESSARIDB_S3_TEST_FFMPEG")
        .map(std::path::PathBuf::from)
        .expect("set TESSARIDB_S3_TEST_FFMPEG to an ffmpeg with libx264, libvpx-vp9 and libopus")
}

/// What ffprobe reports of the first stream of `kind` (`v` or `a`) in `bytes`, as `codec,width,height,pix_fmt`.
async fn probe(bytes: &[u8], kind: &str) -> String {
    let file = std::env::temp_dir().join(format!(
        "tessaridb-s3-probe-{}",
        uuid::Uuid::new_v4().simple()
    ));
    tokio::fs::write(&file, bytes)
        .await
        .expect("probe file written");
    let out = tokio::process::Command::new(ffmpeg().with_file_name("ffprobe"))
        .args([
            "-v",
            "error",
            "-select_streams",
            &format!("{kind}:0"),
            "-show_entries",
        ])
        .args(["stream=codec_name,width,height,pix_fmt", "-of", "csv=p=0"])
        .arg(&file)
        .output()
        .await
        .expect("ffprobe ran");
    tokio::fs::remove_file(&file)
        .await
        .expect("probe file removed");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .trim_end_matches(',')
        .to_owned()
}

#[tokio::test]
#[ignore = "needs a TessariDB node and ffmpeg: set TESSARIDB_S3_TEST_META, _USER, _PASSWORD and _FFMPEG"]
async fn an_uploaded_video_gets_web_renditions_and_a_poster_through_ffmpeg() {
    use tessari_s3_daemons::{Daemon, MediaPlanner, VideoWorker};
    use tessari_s3_infrastructure::media_video::Ffmpeg;
    let _why = IGNORED;
    let (state, planter) = fresh().await;
    // A two-second 1280×720 clip with a tone, made by the same ffmpeg — in 4:4:4, which a browser does not play, so the
    // renditions' 4:2:0 is the worker's doing.
    let clip = std::env::temp_dir().join(format!(
        "tessaridb-s3-clip-{}.mp4",
        uuid::Uuid::new_v4().simple()
    ));
    let made = tokio::process::Command::new(ffmpeg())
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=1280x720:rate=10:duration=2",
        ])
        .args([
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=2",
            "-c:v",
            "libx264",
            "-pix_fmt",
        ])
        .args(["yuv444p", "-c:a", "aac", "-shortest"])
        .arg(&clip)
        .status()
        .await
        .expect("ffmpeg ran");
    assert!(made.success(), "the clip was made");
    let original = tokio::fs::read(&clip).await.expect("clip read");
    tokio::fs::remove_file(&clip).await.expect("clip removed");
    assert_eq!(call(&state, "PUT", "/films", vec![], b"").await.status, 200);
    planter
        .run(
            "UPDATE buckets:'films' SET media = [{ id: 'clips', content_types: ['video/*'], prefix: 'clips/', \
             outputs: ['video/360p/mp4', 'video/360p/webm', 'poster/320/jpeg'] }];",
            Vec::new(),
        )
        .await
        .expect("media rules set");
    let put = call_with(
        &state,
        "PUT",
        "/films/clips/a.mp4",
        vec![],
        vec![("content-type", "video/mp4")],
        &original,
    )
    .await;
    assert_eq!(put.status, 200);
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
    for _ in 0..3 {
        worker.run_once().await.expect("a video run");
    }
    let at = "/films/clips/a.mp4.renditions";
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
    let mp4 = fetched(format!("{at}/video-360p.mp4"), "video/mp4").await;
    assert_eq!(probe(&mp4, "v").await, "h264,640,360,yuv420p");
    assert_eq!(probe(&mp4, "a").await, "aac");
    let webm = fetched(format!("{at}/video-360p.webm"), "video/webm").await;
    assert_eq!(probe(&webm, "v").await, "vp9,640,360,yuv420p");
    assert_eq!(probe(&webm, "a").await, "opus");
    let poster = fetched(format!("{at}/poster-320.jpg"), "image/jpeg").await;
    assert_eq!(
        image::guess_format(&poster).ok(),
        Some(image::ImageFormat::Jpeg)
    );
    let frame = image::load_from_memory(&poster).expect("a poster");
    assert_eq!((frame.width(), frame.height()), (320, 180));
    assert!(
        in_queue(&planter, "media_videos", "output")
            .await
            .is_empty(),
        "every task is settled"
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_restarted_node_takes_back_the_tasks_its_killed_worker_held() {
    use tessari_s3_core::media::Output;
    use tessari_s3_storage::media::{Rendition, TaskQueue};
    let _why = IGNORED;
    let (state, planter) = fresh().await;
    let media = state.storage().media();
    media
        .enqueue(&Rendition {
            bucket: "pictures".to_owned(),
            key: "a.png".to_owned(),
            version_id: None,
            source: [9; 16],
            output: Output::parse("image/320/jpeg").expect("an output"),
        })
        .await
        .expect("queued");
    // A worker takes the task and is killed before settling it: the hold carries a deadline, so it lapses on its own.
    let held = media
        .claim(TaskQueue::Images, "node-a", 1)
        .await
        .expect("claimed");
    assert_eq!(held.len(), 1);
    assert!(
        matches!(
            in_queue(&planter, "media_images", "claimed_until")
                .await
                .as_slice(),
            [Value::Datetime { .. }]
        ),
        "the hold has a deadline"
    );
    assert!(
        media
            .claim(TaskQueue::Images, "node-a", 1)
            .await
            .expect("claimed")
            .is_empty()
    );
    // The node comes back under the same name and takes back what its predecessor held, without waiting the hold out.
    media
        .reclaim(TaskQueue::Images, "node-a")
        .await
        .expect("reclaimed");
    let again = media
        .claim(TaskQueue::Images, "node-a", 1)
        .await
        .expect("claimed");
    assert_eq!(again.len(), 1, "the task is claimable again at once");
    assert_eq!(again[0].attempts, 2);
}
