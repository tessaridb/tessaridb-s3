//! A store a build without versions wrote, converted at start against a real node: each object becomes its key's null
//! version, an object a versioned build already wrote is left as it is, and the event that released a replaced
//! object's parts is gone, so the parts a noncurrent version owns survive an overwrite.

use tessari_s3_infrastructure::tessaridb::{Answer, MetaPool, Value};

use crate::large::files;
use crate::versions_kept::{put, set_versioning, version_header, versions};
use crate::{IGNORED, call, fresh_with, scratch_dir};

/// What a build without versions left behind: no version records, no null pointers, no conversion mark, and the
/// event that released a replaced object's parts.
async fn as_before_versions(planter: &MetaPool) {
    planter
        .run(
            "BEGIN; DELETE FROM versions WHERE seq >= 1 LIMIT ALL; \
             DELETE FROM version_nulls WHERE seq >= 1 LIMIT ALL; \
             DELETE s3_conversions:'versions'; \
             DEFINE EVENT parts_released ON objects FOR UPDATE, DELETE \
             WHEN $before.upload != NONE AND $before.upload != $after.upload \
             THEN { LET $old = $before.upload; DELETE FROM parts WHERE upload = $old LIMIT ALL; }; COMMIT;",
            Vec::new(),
        )
        .await
        .expect("the store is set back");
}

async fn objects_definition(planter: &MetaPool) -> String {
    match planter
        .run("INFO FOR TABLE objects;", Vec::new())
        .await
        .expect("table info")
        .into_iter()
        .next()
    {
        Some(Answer::Value {
            value: Value::Object(info),
            ..
        }) => match info.get("definition") {
            Some(Value::String(definition)) => definition.clone(),
            other => panic!("definition: {other:?}"),
        },
        other => panic!("table info: {other:?}"),
    }
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_store_from_before_versions_gets_its_null_versions_at_start() {
    let _why = IGNORED;
    let dir = scratch_dir();
    let (state, planter, _) = fresh_with(dir.clone()).await;
    assert_eq!(call(&state, "PUT", "/files", vec![], b"").await.status, 200);
    put(&state, "/files/old", b"written before versions").await;
    as_before_versions(&planter).await;
    assert!(versions(&planter, "files", "old").await.is_empty());
    assert!(
        objects_definition(&planter)
            .await
            .contains("parts_released")
    );
    // Written after the store was set back, as a versioned build would: it holds its null version already.
    put(&state, "/files/new", b"written with versions").await;

    let objects = state.storage().objects();
    assert_eq!(
        objects
            .convert_versions()
            .await
            .expect("the start converts"),
        1
    );
    assert_eq!(objects.convert_versions().await.expect("once"), 0);

    let old = versions(&planter, "files", "old").await;
    assert_eq!(old.len(), 1, "{old:?}");
    assert_eq!((old[0].version.as_str(), old[0].seq), ("null", 1));
    assert!(!old[0].marker && old[0].data.is_some());
    let new = versions(&planter, "files", "new").await;
    assert_eq!(new.len(), 1, "converted at most once: {new:?}");
    assert!(
        !objects_definition(&planter)
            .await
            .contains("parts_released"),
        "the versions own a replaced object's parts now"
    );
    let read = call(&state, "GET", "/files/old", vec![], b"").await;
    assert_eq!(read.body, "written before versions");
    assert_eq!(version_header(&read), None);

    // The converted object is a version like any other: it becomes noncurrent and keeps its bytes.
    set_versioning(&state, "files", "Enabled").await;
    put(&state, "/files/old", b"newer").await;
    let old = versions(&planter, "files", "old").await;
    assert_eq!(
        old.iter().map(|version| version.seq).collect::<Vec<_>>(),
        vec![2, 1]
    );
    assert!(state.storage().objects().reclaim(0, 1000).await.is_ok());
    assert_eq!(
        files(&dir).len(),
        3,
        "both versions of `old` and `new` keep their files"
    );
}
