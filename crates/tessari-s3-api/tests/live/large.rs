//! Objects above the inline size: data files on the node's drive, the `gc` queue in TessariDB, bitrot on read.

use std::path::{Path, PathBuf};

use tessari_s3_core::objects::checksum::Hashes;
use tessari_s3_infrastructure::tessaridb::{Answer, MetaPool, Value};

use crate::{IGNORED, Seen, call, call_with, fresh_with, scratch_dir};

const MIB: usize = 1 << 20;

fn header<'a>(seen: &'a Seen, name: &str) -> Option<&'a str> {
    seen.headers.get(name).and_then(|v| v.to_str().ok())
}

/// `size` bytes that differ block to block, so a misplaced block cannot read back as the right one.
pub(crate) fn body(size: usize, seed: u8) -> Vec<u8> {
    (0..size)
        .map(|i| {
            u8::try_from(
                i.wrapping_div(7)
                    .wrapping_add(usize::from(seed))
                    .wrapping_rem(251),
            )
            .expect("small")
        })
        .collect()
}

fn etag(bytes: &[u8]) -> String {
    let mut hashes = Hashes::new();
    hashes.update(bytes);
    format!("\"{}\"", hashes.finish().md5_hex)
}

async fn records(planter: &MetaPool, script: &str, parameters: Vec<(String, Value)>) -> Vec<Value> {
    match planter
        .run(script, parameters)
        .await
        .expect("query")
        .into_iter()
        .next()
    {
        Some(Answer::Records { records, .. }) => records.into_iter().map(|(_, v)| v).collect(),
        Some(Answer::Value {
            value: Value::None, ..
        })
        | None => Vec::new(),
        Some(Answer::Value { value, .. }) => vec![value],
        Some(_) => panic!("unexpected answer"),
    }
}

/// The data ids queued in `gc`, sorted.
pub(crate) async fn queued(planter: &MetaPool) -> Vec<[u8; 16]> {
    let mut ids: Vec<[u8; 16]> = records(planter, "SELECT * FROM gc;", Vec::new())
        .await
        .into_iter()
        .map(|record| match record {
            Value::Object(fields) => match fields.get("data") {
                Some(Value::Uuid(id)) => *id,
                other => panic!("gc record without a data id: {other:?}"),
            },
            other => panic!("gc record is not an object: {other:?}"),
        })
        .collect();
    ids.sort_unstable();
    ids
}

/// The data id the object record at `bucket/key` points to.
pub(crate) async fn data_of(planter: &MetaPool, bucket: &str, key: &str) -> Option<[u8; 16]> {
    let id = vec![("id".to_owned(), Value::String(format!("{bucket}/{key}")))];
    records(planter, "SELECT * FROM ONLY objects:$id;", id)
        .await
        .into_iter()
        .next()
        .and_then(|record| match record {
            Value::Object(fields) => match fields.get("data") {
                Some(Value::Uuid(id)) => Some(*id),
                _ => None,
            },
            _ => None,
        })
}

/// Every data file under `dir` (temporary names included).
pub(crate) fn files(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&next) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

async fn bucket(state: &tessari_s3_api::ApiState, name: &str) {
    assert_eq!(
        call(state, "PUT", &format!("/{name}"), vec![], b"")
            .await
            .status,
        200
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn large_objects_round_trip_through_data_files() {
    let _why = IGNORED;
    let dir = scratch_dir();
    let (state, planter, _) = fresh_with(Some(dir.clone())).await;
    bucket(&state, "large").await;
    for (key, size) in [
        ("one", MIB + 5),
        ("five", 5 * MIB + 3),
        ("edge", 128 * 1024 + 1),
    ] {
        let bytes = body(size, 3);
        let put = call(&state, "PUT", &format!("/large/{key}"), vec![], &bytes).await;
        assert_eq!(put.status, 200, "{key}: {}", put.body);
        assert_eq!(header(&put, "etag"), Some(etag(&bytes).as_str()), "{key}");
        let got = call(&state, "GET", &format!("/large/{key}"), vec![], b"").await;
        assert_eq!(got.status, 200, "{key}");
        assert!(
            got.bytes == bytes,
            "{key}: the bytes read back are the bytes written"
        );
        assert_eq!(
            header(&got, "content-length"),
            Some(size.to_string().as_str())
        );
        assert!(
            data_of(&planter, "large", key).await.is_some(),
            "{key} is a data object"
        );
    }
    // A range across the first block boundary, served from two verified blocks.
    let five = body(5 * MIB + 3, 3);
    let (start, end) = (MIB - 10, MIB + 9);
    let ranged = call_with(
        &state,
        "GET",
        "/large/five",
        vec![],
        vec![("range", &format!("bytes={start}-{end}"))],
        b"",
    )
    .await;
    assert_eq!(ranged.status, 206);
    assert!(ranged.bytes == five[start..=end], "the bytes of the range");
    assert_eq!(
        header(&ranged, "content-range"),
        Some(format!("bytes {start}-{end}/{}", five.len()).as_str())
    );
    let head = call(&state, "HEAD", "/large/five", vec![], b"").await;
    assert_eq!((head.status, head.bytes.len()), (200, 0));
    assert_eq!(
        header(&head, "content-length"),
        Some((5 * MIB + 3).to_string().as_str())
    );
    assert!(
        queued(&planter).await.is_empty(),
        "every committed write took its pending entry with it"
    );
    std::fs::remove_dir_all(&dir).expect("cleanup");
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn overwrite_and_delete_queue_the_superseded_data_in_the_same_commit() {
    let dir = scratch_dir();
    let (state, planter, _) = fresh_with(Some(dir.clone())).await;
    bucket(&state, "turn").await;
    assert_eq!(
        call(&state, "PUT", "/turn/k", vec![], &body(MIB + 1, 1))
            .await
            .status,
        200
    );
    let first = data_of(&planter, "turn", "k").await.expect("data");
    assert_eq!(
        call(&state, "PUT", "/turn/k", vec![], &body(MIB + 2, 2))
            .await
            .status,
        200
    );
    let second = data_of(&planter, "turn", "k").await.expect("data");
    assert_ne!(first, second, "an overwrite writes a new data file");
    assert_eq!(
        queued(&planter).await,
        vec![first],
        "the superseded data is queued"
    );
    assert_eq!(
        call(&state, "PUT", "/turn/k", vec![], b"small now")
            .await
            .status,
        200
    );
    let mut both = vec![first, second];
    both.sort_unstable();
    assert_eq!(
        queued(&planter).await,
        both,
        "replacing data with inline bytes queues it too"
    );
    assert_eq!(
        call(&state, "PUT", "/turn/d", vec![], &body(MIB, 4))
            .await
            .status,
        200
    );
    let third = data_of(&planter, "turn", "d").await.expect("data");
    assert_eq!(
        call(&state, "DELETE", "/turn/d", vec![], b"").await.status,
        204
    );
    let mut all = vec![first, second, third];
    all.sort_unstable();
    assert_eq!(queued(&planter).await, all, "a delete queues the data");
    std::fs::remove_dir_all(&dir).expect("cleanup");
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_large_write_that_does_not_commit_leaves_no_file_and_no_pending_entry() {
    let dir = scratch_dir();
    let (state, planter, _) = fresh_with(Some(dir.clone())).await;
    bucket(&state, "keep").await;
    let original = body(MIB + 7, 9);
    assert_eq!(
        call(&state, "PUT", "/keep/k", vec![], &original)
            .await
            .status,
        200
    );
    let before = files(&dir);
    assert_eq!(before.len(), 1, "one committed data file");
    let refused = call_with(
        &state,
        "PUT",
        "/keep/k",
        vec![],
        vec![("if-none-match", "*")],
        &body(MIB + 1, 5),
    )
    .await;
    assert_eq!(
        (refused.status, refused.code.as_deref()),
        (412, Some("PreconditionFailed"))
    );
    let digest = call_with(
        &state,
        "PUT",
        "/keep/other",
        vec![],
        vec![("content-md5", "1B2M2Y8AsgTpgAmY7PhCfg==")],
        &body(MIB + 1, 6),
    )
    .await;
    assert_eq!(
        (digest.status, digest.code.as_deref()),
        (400, Some("BadDigest"))
    );
    assert_eq!(files(&dir), before, "neither write left a file");
    assert!(queued(&planter).await.is_empty(), "nor a pending entry");
    assert!(call(&state, "GET", "/keep/k", vec![], b"").await.bytes == original);
    std::fs::remove_dir_all(&dir).expect("cleanup");
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_damaged_block_is_never_served() {
    let dir = scratch_dir();
    let (state, _, _) = fresh_with(Some(dir.clone())).await;
    bucket(&state, "rot").await;
    let bytes = body(3 * MIB, 7);
    assert_eq!(
        call(&state, "PUT", "/rot/first", vec![], &bytes)
            .await
            .status,
        200
    );
    let path = files(&dir).pop().expect("a data file");
    let mut raw = std::fs::read(&path).expect("read");
    raw[32 + 100] ^= 0x01;
    std::fs::write(&path, &raw).expect("flip a bit in block 0");
    let first = call(&state, "GET", "/rot/first", vec![], b"").await;
    assert_eq!(
        (first.status, first.code.as_deref()),
        (500, Some("InternalError")),
        "damage in the first block is caught before the response starts"
    );
    raw[32 + 100] ^= 0x01;
    raw[32 + 2 * (MIB + 32) + 5] ^= 0x01;
    std::fs::write(&path, &raw).expect("restore block 0, flip a bit in block 2");
    let later = call(&state, "GET", "/rot/first", vec![], b"").await;
    assert_eq!(later.status, 200, "the head was sent with block 0");
    assert!(
        later.bytes.len() < bytes.len(),
        "the body stops at the damaged block"
    );
    std::fs::remove_dir_all(&dir).expect("cleanup");
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn without_a_data_directory_a_large_object_is_refused_by_name() {
    let (state, _, _) = fresh_with(None).await;
    bucket(&state, "nodir").await;
    let refused = call(
        &state,
        "PUT",
        "/nodir/k",
        vec![],
        &vec![b'x'; 128 * 1024 + 1],
    )
    .await;
    assert_eq!(
        (refused.status, refused.code.as_deref()),
        (501, Some("NotImplemented"))
    );
    assert!(
        refused.body.contains("TESSARIDB_S3_DATA_DIR"),
        "{}",
        refused.body
    );
    assert_eq!(
        call(
            &state,
            "PUT",
            "/nodir/edge",
            vec![],
            &vec![b'x'; 128 * 1024]
        )
        .await
        .status,
        200,
        "128 KiB is inline and needs no data directory"
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn an_upload_cut_off_before_its_commit_leaves_a_queued_file_and_no_object() {
    let dir = scratch_dir();
    let (state, planter, _) = fresh_with(Some(dir.clone())).await;
    let objects = state.storage().objects();
    let bytes = body(MIB + 3, 2);
    let mut upload = objects
        .upload(u64::try_from(bytes.len()).expect("small"))
        .await
        .expect("upload starts");
    let id = upload.id();
    assert_eq!(
        queued(&planter).await,
        vec![id],
        "queued before a byte reaches the disk"
    );
    upload.append(&bytes).await.expect("append");
    upload.finish().await.expect("the file is durable");
    // The process dies here: the object record was never committed.
    assert_eq!(files(&dir).len(), 1, "the file is on disk");
    assert_eq!(
        queued(&planter).await,
        vec![id],
        "and its id is queued for the reclaimer"
    );
    assert!(
        records(&planter, "SELECT * FROM objects;", Vec::new())
            .await
            .is_empty(),
        "no object points at it"
    );
    std::fs::remove_dir_all(&dir).expect("cleanup");
}
