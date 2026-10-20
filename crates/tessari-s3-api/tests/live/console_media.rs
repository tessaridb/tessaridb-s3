//! A bucket's media rules through the console, against a real node (G004 C1): whoever configures the bucket sets them,
//! anyone who reads it sees them, a rule the server cannot honour is refused with nothing stored, and an empty list
//! turns media processing off; an output names any side or a box with a mode and a quality, and is made so.

use serde_json::{Value, json};
use tessari_s3_core::authz::{Access, Role};

use crate::access::{grant, user_with_key};
use crate::console::{send, sign_in};
use crate::console_users::{sign_in_as, states};
use crate::{IGNORED, call_as};

const MEDIA: &str = "/api/v1/buckets/pics/media";

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn the_bucket_s_configurer_sets_media_rules_and_a_bad_rule_is_refused() {
    let _why = IGNORED;
    let (state, console, _) = states().await;
    let ann = user_with_key(state.storage(), "ann", "alpha", Role::Member, true).await;
    let bob = user_with_key(state.storage(), "bob", "alpha", Role::Member, false).await;
    assert_eq!(
        call_as(&state, ann.pair(), "PUT", "/pics", vec![], vec![], b"")
            .await
            .status,
        200
    );
    // Bob may read and write the bucket's objects, which is everything a grant confers, and still may not configure it.
    grant(
        state.storage(),
        "bob",
        "pics",
        Access {
            read: true,
            write: true,
        },
    )
    .await;
    let ann_token = sign_in_as(&console, &ann).await.expect("ann signed in");
    let bob_token = sign_in_as(&console, &bob).await.expect("bob signed in");
    let none = send(&console, "GET", MEDIA, Some(&ann_token), None).await;
    assert_eq!(none.status, 200);
    assert_eq!(none.json()["rules"], json!([]));

    let rules = json!({"rules": [
        {"id": "photos", "content_types": ["image/*"], "prefix": "photos/", "outputs": ["image/320/jpeg", "image/1920/webp"]},
        {"id": "clips", "content_types": ["video/mp4"], "prefix": "", "outputs": ["video/720p/mp4", "poster/640/jpeg"]}
    ]});
    assert_eq!(
        send(&console, "GET", MEDIA, Some(&bob_token), None)
            .await
            .status,
        200,
        "a reader of the bucket sees its media rules"
    );
    let refused = send(
        &console,
        "PUT",
        MEDIA,
        Some(&bob_token),
        Some(&rules.to_string()),
    )
    .await;
    assert_eq!(
        refused.status, 403,
        "a grant to read and write does not configure the bucket"
    );
    let set = send(
        &console,
        "PUT",
        MEDIA,
        Some(&ann_token),
        Some(&rules.to_string()),
    )
    .await;
    assert_eq!(set.status, 204, "{:?}", set.code());
    let got = send(&console, "GET", MEDIA, Some(&ann_token), None).await;
    assert_eq!(got.json()["rules"], rules["rules"]);

    for (body, code) in [
        (
            json!({"rules": [{"id": "a", "content_types": ["image/png"], "prefix": "", "outputs": ["video/720p/mp4"]}]}),
            "invalid_media",
        ),
        (
            json!({"rules": [{"id": "a", "content_types": ["image/png"], "prefix": "", "outputs": ["image/1280/gif"]}]}),
            "invalid_media",
        ),
        (json!({"rules": [], "space": "beta"}), "bad_request"),
    ] {
        let seen = send(
            &console,
            "PUT",
            MEDIA,
            Some(&ann_token),
            Some(&body.to_string()),
        )
        .await;
        assert_eq!(seen.status, 400, "{body}");
        assert_eq!(seen.code().as_deref(), Some(code), "{body}");
    }
    assert_eq!(
        send(&console, "GET", MEDIA, Some(&ann_token), None)
            .await
            .json()["rules"],
        rules["rules"],
        "nothing refused was stored"
    );
    let off = send(
        &console,
        "PUT",
        MEDIA,
        Some(&ann_token),
        Some(&json!({"rules": []}).to_string()),
    )
    .await;
    assert_eq!(off.status, 204);
    let cleared: Value = send(&console, "GET", MEDIA, Some(&ann_token), None)
        .await
        .json();
    assert_eq!(cleared["rules"], json!([]));
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn an_operator_sees_the_renditions_that_failed_for_good_and_a_member_does_not() {
    use tessari_s3_core::media::Output;
    use tessari_s3_storage::media::{Rendition, TaskQueue};
    let _why = IGNORED;
    let (state, console, _) = states().await;
    let media = state.storage().media();
    media
        .enqueue(&Rendition {
            bucket: "pictures".to_owned(),
            key: "a.png".to_owned(),
            version_id: None,
            source: [5; 16],
            output: Output::parse("image/320/jpeg").expect("an output"),
        })
        .await
        .expect("queued");
    // Five hand-outs, the last one failing: the task stops being handed out and stays as the failure.
    for _ in 0..4 {
        assert_eq!(
            media
                .claim(TaskQueue::Images, "node-a", 1)
                .await
                .expect("claimed")
                .len(),
            1
        );
        media
            .reclaim(TaskQueue::Images, "node-a")
            .await
            .expect("freed");
    }
    let last = media
        .claim(TaskQueue::Images, "node-a", 1)
        .await
        .expect("claimed");
    media
        .retry(&last[0], "the original is not an image this server reads")
        .await
        .expect("failed");
    let operator = sign_in(&console).await;
    let listed = send(
        &console,
        "GET",
        "/api/v1/media/failed",
        Some(&operator),
        None,
    )
    .await;
    assert_eq!(listed.status, 200);
    assert_eq!(
        listed.json(),
        json!({"failed": [{
            "queue": "images", "bucket": "pictures", "key": "a.png", "version_id": null, "output": "image/320/jpeg",
            "attempts": 5, "error": "the original is not an image this server reads"
        }], "more": false})
    );
    let ann = user_with_key(state.storage(), "ann", "alpha", Role::Member, true).await;
    let member = sign_in_as(&console, &ann).await.expect("ann signed in");
    assert_eq!(
        send(&console, "GET", "/api/v1/media/failed", Some(&member), None)
            .await
            .code()
            .as_deref(),
        Some("forbidden"),
        "the failures span every space, so only an operator reads them"
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn an_output_names_a_box_a_mode_and_a_quality_and_is_rendered_so() {
    use tessari_s3_daemons::{Daemon, ImageWorker, MediaPlanner};

    use crate::media::{image_at, png};
    let _why = IGNORED;
    let (state, console, _) = states().await;
    let ann = user_with_key(state.storage(), "ann", "alpha", Role::Member, true).await;
    assert_eq!(
        call_as(&state, ann.pair(), "PUT", "/pics", vec![], vec![], b"")
            .await
            .status,
        200
    );
    let token = sign_in_as(&console, &ann).await.expect("ann signed in");
    for (output, message) in [
        ("image/640/png@80", "png is lossless and takes no quality"),
        (
            "image/640-cover/webp",
            "fit, cover and exact follow a box such as 400x400",
        ),
        (
            "image/8000/webp",
            "an image side is between 16 and 7680 pixels",
        ),
        ("image/640/jpeg@0", "a quality is between 1 and 100"),
        (
            "image/7680x7680-exact/png",
            "a box holds at most 50 000 000 pixels",
        ),
    ] {
        let body = json!({"rules": [{"id": "a", "content_types": ["image/*"], "prefix": "", "outputs": [output]}]});
        let seen = send(
            &console,
            "PUT",
            MEDIA,
            Some(&token),
            Some(&body.to_string()),
        )
        .await;
        assert_eq!(seen.status, 400, "{output}");
        assert_eq!(seen.code().as_deref(), Some("invalid_media"), "{output}");
        assert_eq!(seen.json()["message"], json!(message), "{output}");
    }
    let outputs = [
        "image/400x400-cover/webp",
        "image/800x600/jpeg",
        "image/300x500-exact/png",
        "image/1300/jpeg@40",
    ];
    let rules = json!({"rules": [{"id": "photos", "content_types": ["image/*"], "prefix": "", "outputs": outputs}]});
    let set = send(
        &console,
        "PUT",
        MEDIA,
        Some(&token),
        Some(&rules.to_string()),
    )
    .await;
    assert_eq!(set.status, 204, "{:?}", set.code());
    let got = send(&console, "GET", MEDIA, Some(&token), None).await;
    assert_eq!(got.json()["rules"], rules["rules"], "stored as spelled");
    let put = call_as(
        &state,
        ann.pair(),
        "PUT",
        "/pics/a.png",
        vec![],
        vec![("content-type", "image/png")],
        &png(2000, 1000),
    )
    .await;
    assert_eq!(put.status, 200);
    let (media, objects) = (state.storage().media(), state.storage().objects().clone());
    media.define().await.expect("group defined");
    MediaPlanner::new(media.clone(), objects.clone())
        .run_once()
        .await
        .expect("planned");
    ImageWorker::new(media, objects, "node-a".to_owned())
        .run_once()
        .await
        .expect("rendered");
    let at = "/pics/a.png.renditions";
    for (file, content_type, format, size) in [
        (
            "image-400x400-cover.webp",
            "image/webp",
            image::ImageFormat::WebP,
            (400, 400),
        ),
        (
            "image-800x600.jpg",
            "image/jpeg",
            image::ImageFormat::Jpeg,
            (800, 400),
        ),
        (
            "image-300x500-exact.png",
            "image/png",
            image::ImageFormat::Png,
            (300, 500),
        ),
        (
            "image-1300-q40.jpg",
            "image/jpeg",
            image::ImageFormat::Jpeg,
            (1300, 650),
        ),
    ] {
        assert_eq!(
            image_at(&state, &format!("{at}/{file}"), content_type, format).await,
            size,
            "{file}"
        );
    }
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_sweep_renders_the_objects_written_before_a_rule_once_a_page_at_a_time() {
    use tessari_s3_daemons::{Daemon, ImageWorker, MediaPlanner};

    use crate::media::{image_at, png};
    const SWEEP: &str = "/api/v1/buckets/pics/media/sweep";
    let _why = IGNORED;
    let (state, console, _) = states().await;
    let ann = user_with_key(state.storage(), "ann", "alpha", Role::Member, true).await;
    let put = |path: String, content_type: &'static str, body: Vec<u8>| {
        let state = &state;
        let pair = ann.pair();
        async move {
            call_as(
                state,
                pair,
                "PUT",
                &path,
                vec![],
                vec![("content-type", content_type)],
                &body,
            )
            .await
            .status
        }
    };
    assert_eq!(put("/pics".to_owned(), "text/plain", Vec::new()).await, 200);
    for name in ["a", "b", "c"] {
        assert_eq!(
            put(format!("/pics/{name}.png"), "image/png", png(400, 200)).await,
            200
        );
    }
    // More than one page: 104 objects, of which three are images.
    for n in 0..101 {
        assert_eq!(
            put(format!("/pics/t/{n:03}"), "text/plain", b"x".to_vec()).await,
            200
        );
    }
    let (media, objects) = (state.storage().media(), state.storage().objects().clone());
    media.define().await.expect("group defined");
    let planner = MediaPlanner::new(media.clone(), objects.clone());
    // Without rules the created objects are settled unplanned: only a sweep reaches them now.
    planner.run_once().await.expect("planned");
    let token = sign_in_as(&console, &ann).await.expect("ann signed in");
    let progress = || async {
        let seen = send(&console, "GET", SWEEP, Some(&token), None).await;
        assert_eq!(seen.status, 200);
        seen.json()
    };
    assert_eq!(
        progress().await,
        json!({"state": "none", "listed": 0, "queued": 0})
    );
    let rules = json!({"rules": [{"id": "p", "content_types": ["image/*"], "prefix": "",
        "outputs": ["image/320/jpeg", "image/64x64-cover/webp"]}]});
    let set = send(
        &console,
        "PUT",
        MEDIA,
        Some(&token),
        Some(&rules.to_string()),
    )
    .await;
    assert_eq!(set.status, 204);
    let start = || async {
        let seen = send(&console, "POST", SWEEP, Some(&token), None).await;
        assert_eq!(seen.status, 202, "{:?}", seen.code());
    };
    start().await;
    assert_eq!(
        progress().await,
        json!({"state": "running", "listed": 0, "queued": 0})
    );
    planner.run_once().await.expect("first page");
    assert_eq!(
        progress().await,
        json!({"state": "running", "listed": 100, "queued": 6})
    );
    // Started again mid-way, it begins from the first key.
    start().await;
    planner.run_once().await.expect("first page again");
    assert_eq!(
        progress().await,
        json!({"state": "running", "listed": 100, "queued": 6})
    );
    planner.run_once().await.expect("last page");
    assert_eq!(
        progress().await,
        json!({"state": "done", "listed": 104, "queued": 6})
    );
    let worker = ImageWorker::new(media.clone(), objects.clone(), "node-a".to_owned());
    for _ in 0..3 {
        worker.run_once().await.expect("rendered");
    }
    for name in ["a", "b", "c"] {
        let at = format!("/pics/{name}.png.renditions");
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
                &format!("{at}/image-64x64-cover.webp"),
                "image/webp",
                image::ImageFormat::WebP
            )
            .await,
            (64, 64)
        );
    }
    // A repeat lists the same objects and queues nothing: every rendition was written since its original.
    start().await;
    planner.run_once().await.expect("first page");
    planner.run_once().await.expect("last page");
    assert_eq!(
        progress().await,
        json!({"state": "done", "listed": 104, "queued": 0})
    );
    let member = user_with_key(state.storage(), "bob", "alpha", Role::Member, false).await;
    let bob = sign_in_as(&console, &member).await.expect("bob signed in");
    assert_eq!(
        send(&console, "POST", SWEEP, Some(&bob), None).await.status,
        403,
        "only whoever configures the bucket starts a sweep"
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_save_naming_a_version_set_again_since_is_refused_and_changes_nothing() {
    let _why = IGNORED;
    let (state, console, _) = states().await;
    let ann = user_with_key(state.storage(), "ann", "alpha", Role::Member, true).await;
    assert_eq!(
        call_as(&state, ann.pair(), "PUT", "/pics", vec![], vec![], b"")
            .await
            .status,
        200
    );
    let token = sign_in_as(&console, &ann).await.expect("ann signed in");
    let read = || async {
        send(&console, "GET", MEDIA, Some(&token), None)
            .await
            .json()
    };
    assert_eq!(read().await["version"], json!(0));
    let rules = |output: &str| json!([{"id": "p", "content_types": ["image/*"], "prefix": "", "outputs": [output]}]);
    let (console, token) = (&console, &token);
    let save = |body: Value| async move {
        send(
            console,
            "PUT",
            MEDIA,
            Some(token),
            Some(&body.to_string()),
        )
        .await
    };
    // Two editors read version 0; the first save wins and moves it to 1.
    let first = save(json!({"rules": rules("image/320/jpeg"), "version": 0})).await;
    assert_eq!(first.status, 204);
    assert_eq!(read().await["version"], json!(1));
    let second = save(json!({"rules": rules("image/640/webp"), "version": 0})).await;
    assert_eq!(second.status, 412);
    assert_eq!(second.code().as_deref(), Some("stale_media"));
    assert_eq!(
        read().await["rules"],
        rules("image/320/jpeg"),
        "the stale save changed nothing"
    );
    // Read again, the second editor saves at the current version; a save naming none is not checked.
    assert_eq!(
        save(json!({"rules": rules("image/640/webp"), "version": 1}))
            .await
            .status,
        204
    );
    assert_eq!(save(json!({"rules": []})).await.status, 204);
    assert_eq!(read().await, json!({"rules": [], "version": 3}));
}
