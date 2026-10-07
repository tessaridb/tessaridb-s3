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
