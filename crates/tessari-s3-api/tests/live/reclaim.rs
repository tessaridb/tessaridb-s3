//! Reclaiming data files: only what the `reclaims` queue hands out, only past the grace period, never a file an
//! object references, and a commit that loses the race to the reclaimer fails instead of pointing at nothing.

use std::collections::BTreeMap;

use tessari_s3_infrastructure::tessaridb::{Answer, MetaPool, Number, Value};
use tessari_s3_storage::objects::{Content, NewObject, WriteCondition, Written};
use tessari_s3_types::{BucketName, ObjectKey};

use crate::large::{body, data_of, files, queued};
use crate::{IGNORED, call, fresh_with, scratch_dir};

const MIB: usize = 1 << 20;

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn queued_files_are_reclaimed_and_live_ones_are_not() {
    let _why = IGNORED;
    let dir = scratch_dir();
    let (state, planter, _) = fresh_with(dir.clone()).await;
    let objects = state.storage().objects();
    assert_eq!(call(&state, "PUT", "/keep", vec![], b"").await.status, 200);
    assert_eq!(
        call(&state, "PUT", "/keep/k", vec![], &body(MIB + 1, 1))
            .await
            .status,
        200
    );
    let first = data_of(&planter, "keep", "k").await.expect("data");
    let current = body(MIB + 2, 2);
    assert_eq!(
        call(&state, "PUT", "/keep/k", vec![], &current)
            .await
            .status,
        200
    );
    let bytes = body(MIB, 3);
    let mut orphan = objects
        .upload(u64::try_from(bytes.len()).expect("small"))
        .await
        .expect("upload");
    orphan.append(&bytes).await.expect("append");
    let orphan = orphan.finish().await.expect("finish").id;
    assert_eq!(files(&dir).len(), 3, "first, current and the orphan");
    let mut both = vec![first, orphan];
    both.sort_unstable();
    assert_eq!(queued(&planter).await, both);

    let reclaimed = objects.reclaim(0, 1000).await.expect("reclaim");
    assert_eq!((reclaimed.examined, reclaimed.removed), (2, 2));
    assert_eq!(files(&dir).len(), 1, "only the referenced file is left");
    assert!(queued(&planter).await.is_empty());
    let got = call(&state, "GET", "/keep/k", vec![], b"").await;
    assert!(
        got.status == 200 && got.bytes == current,
        "the current object still reads"
    );
    std::fs::remove_dir_all(&dir).expect("cleanup");
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_commit_that_lost_its_data_to_the_reclaimer_is_refused() {
    let dir = scratch_dir();
    let (state, planter, _) = fresh_with(dir.clone()).await;
    assert_eq!(call(&state, "PUT", "/race", vec![], b"").await.status, 200);
    let objects = state.storage().objects();
    let bytes = body(MIB, 4);
    let mut upload = objects
        .upload(u64::try_from(bytes.len()).expect("small"))
        .await
        .expect("upload");
    upload.append(&bytes).await.expect("append");
    let uploaded = upload.finish().await.expect("finish");
    // The upload outlived the grace period: the reclaimer takes its file before the object commits.
    assert_eq!(objects.reclaim(0, 1000).await.expect("reclaim").removed, 1);
    let object = NewObject {
        size: uploaded.size,
        etag: format!("\"{}\"", uploaded.digests.md5_hex),
        headers: BTreeMap::new(),
        metadata: BTreeMap::new(),
        checksums: BTreeMap::new(),
        content: Content::Data(uploaded.id),
    };
    let bucket = BucketName::new("race").expect("name");
    let key = ObjectKey::new("k").expect("key");
    let written = objects
        .put(&bucket, &key, &object, &WriteCondition::None)
        .await
        .expect("an answer");
    assert_eq!(
        written,
        Written::DataReclaimed,
        "never a record pointing at a removed file"
    );
    assert_eq!(data_of(&planter, "race", "k").await, None);
    assert_eq!(
        call(&state, "GET", "/race/k", vec![], b"")
            .await
            .code
            .as_deref(),
        Some("NoSuchKey")
    );
    std::fs::remove_dir_all(&dir).expect("cleanup");
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_queued_file_a_part_still_references_is_kept() {
    let dir = scratch_dir();
    let (state, planter, _) = fresh_with(dir.clone()).await;
    assert_eq!(
        call(&state, "PUT", "/parted", vec![], b"").await.status,
        200
    );
    let created = call(&state, "POST", "/parted/k", vec![("uploads", None)], b"").await;
    let upload = created
        .body
        .split_once("<UploadId>")
        .and_then(|(_, rest)| rest.split_once("</UploadId>"))
        .map(|(id, _)| id.to_owned())
        .expect("an upload id");
    let part = call(
        &state,
        "PUT",
        "/parted/k",
        vec![
            ("partNumber", Some("1")),
            ("uploadId", Some(upload.as_str())),
        ],
        &body(MIB, 5),
    )
    .await;
    assert_eq!(part.status, 200, "{}", part.body);
    assert_eq!(files(&dir).len(), 1);
    // A stray queue entry for a file a part still holds (a crash between two steps could leave one).
    let answers = planter
        .run("SELECT data FROM parts;", Vec::new())
        .await
        .expect("parts read");
    let Some(tessari_s3_infrastructure::tessaridb::Answer::Records { records, .. }) =
        answers.into_iter().next()
    else {
        panic!("no parts answer");
    };
    let data = match records.first().map(|(_, record)| record) {
        Some(tessari_s3_infrastructure::tessaridb::Value::Object(fields)) => {
            fields.get("data").cloned()
        }
        _ => None,
    }
    .expect("a part's data");
    planter
        .run(
            "UPSERT reclaims:$data SET data = $data, due = time::now();",
            vec![("data".to_owned(), data)],
        )
        .await
        .expect("queue planted");
    let reclaimed = state
        .storage()
        .objects()
        .reclaim(0, 1000)
        .await
        .expect("reclaim");
    assert_eq!(
        (reclaimed.removed, reclaimed.kept),
        (0, 1),
        "the part's file is referenced"
    );
    assert_eq!(files(&dir).len(), 1);
    std::fs::remove_dir_all(&dir).expect("cleanup");
}

/// Seconds from now until the reclamation queue hands data `id` out, as the store computes them.
pub(crate) async fn due_in(planter: &MetaPool, id: [u8; 16]) -> i64 {
    let answers = planter
        .run(
            "SELECT time::unix(due) - time::unix(time::now()) AS ahead FROM ONLY reclaims:$data;",
            vec![("data".to_owned(), Value::Uuid(id))],
        )
        .await
        .expect("a reclaims read");
    match answers.into_iter().next() {
        Some(Answer::Records { records, .. }) => match records.first().map(|(_, record)| record) {
            Some(Value::Object(fields)) => match fields.get("ahead") {
                Some(Value::Number(Number::Integer(ahead))) => *ahead,
                other => panic!("no due instant: {other:?}"),
            },
            other => panic!("no queue record for the data id: {other:?}"),
        },
        other => panic!("not a record answer: {other:?}"),
    }
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_queued_data_id_waits_in_the_reclaim_queue_until_its_grace_ends() {
    let dir = scratch_dir();
    let mut config = crate::live_config(crate::test_node(), None, &dir);
    config.reclaim_grace_secs = 600;
    let storage = crate::prepared(&config).await;
    let planter = MetaPool::new(config.meta.clone()).expect("pool");
    let state =
        tessari_s3_api::ApiState::new(&config, tessari_s3_api::ApiState::system_clock(), storage);
    let objects = state.storage().objects();
    assert_eq!(call(&state, "PUT", "/wait", vec![], b"").await.status, 200);
    assert_eq!(
        call(&state, "PUT", "/wait/k", vec![], &body(MIB + 1, 7))
            .await
            .status,
        200
    );
    let replaced = data_of(&planter, "wait", "k").await.expect("data");
    assert_eq!(
        call(&state, "PUT", "/wait/k", vec![], &body(MIB + 2, 8))
            .await
            .status,
        200
    );
    let bytes = body(MIB, 9);
    let mut orphan = objects
        .upload(u64::try_from(bytes.len()).expect("small"))
        .await
        .expect("upload");
    orphan.append(&bytes).await.expect("append");
    let orphan = orphan.finish().await.expect("finish").id;
    // The overwrite queues through the store's event, the open upload through the server: both wait the grace.
    for id in [replaced, orphan] {
        let ahead = due_in(&planter, id).await;
        assert!(
            (590..=600).contains(&ahead),
            "handed out {ahead} s from now, not after the 600 s grace"
        );
    }
    let early = objects.reclaim(0, 1000).await.expect("reclaim");
    assert_eq!(
        (early.examined, early.removed),
        (0, 0),
        "the queue holds both back"
    );
    assert_eq!(files(&dir).len(), 3);
    std::fs::remove_dir_all(&dir).expect("cleanup");
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_removal_a_dead_reclaimer_marked_is_finished_by_the_next_pass() {
    let dir = scratch_dir();
    let (state, planter, _) = fresh_with(dir.clone()).await;
    let objects = state.storage().objects();
    let bytes = body(MIB, 10);
    let mut upload = objects
        .upload(u64::try_from(bytes.len()).expect("small"))
        .await
        .expect("upload");
    upload.append(&bytes).await.expect("append");
    let id = upload.finish().await.expect("finish").id;
    // A reclaimer took the item and marked it, then died; `RELEASE` hands it back as its lapsed hold would.
    planter
        .run(
            "CLAIM reclaims:$data; \
             UPDATE reclaims:$data SET reclaiming = time::now() WHERE reclaiming = NONE; \
             RELEASE reclaims:$data;",
            vec![("data".to_owned(), Value::Uuid(id))],
        )
        .await
        .expect("a dead reclaimer's claim and mark");
    let done = objects.reclaim(0, 1000).await.expect("reclaim");
    assert_eq!((done.examined, done.removed), (1, 1));
    assert!(files(&dir).is_empty(), "the marked file is removed");
    assert!(queued(&planter).await.is_empty());
    std::fs::remove_dir_all(&dir).expect("cleanup");
}
