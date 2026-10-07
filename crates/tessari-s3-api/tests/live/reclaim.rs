//! Reclaiming data files: only what the `gc` queue names, only past the grace period, never a file an object
//! references, and a commit that loses the race to the reclaimer fails instead of pointing at nothing.

use std::collections::BTreeMap;

use tessari_s3_storage::objects::{Content, NewObject, WriteCondition, Written};
use tessari_s3_types::{BucketName, ObjectKey};

use crate::large::{body, data_of, files, queued};
use crate::{IGNORED, call, fresh_with, scratch_dir};

const MIB: usize = 1 << 20;

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn queued_files_are_reclaimed_after_the_grace_period_and_live_ones_are_not() {
    let _why = IGNORED;
    let dir = scratch_dir();
    let (state, planter, _) = fresh_with(Some(dir.clone())).await;
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
    let mut orphan = objects.upload().await.expect("upload");
    orphan.append(&body(MIB, 3)).await.expect("append");
    let orphan = orphan.finish().await.expect("finish").id;
    assert_eq!(files(&dir).len(), 3, "first, current and the orphan");
    let mut both = vec![first, orphan];
    both.sort_unstable();
    assert_eq!(queued(&planter).await, both);

    let patient = objects.reclaim(3600, 1000).await.expect("reclaim");
    assert_eq!(patient.removed, 0, "nothing is older than an hour");
    assert_eq!(files(&dir).len(), 3);

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
    let (state, planter, _) = fresh_with(Some(dir.clone())).await;
    assert_eq!(call(&state, "PUT", "/race", vec![], b"").await.status, 200);
    let objects = state.storage().objects();
    let mut upload = objects.upload().await.expect("upload");
    upload.append(&body(MIB, 4)).await.expect("append");
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
    let (state, planter, _) = fresh_with(Some(dir.clone())).await;
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
            "UPSERT gc:$data SET data = $data, queued = time::now();",
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
