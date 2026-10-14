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
