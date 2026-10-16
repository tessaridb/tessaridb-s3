//! Rotating the server key (G017 W6): a new server key becomes the one new data keys are wrapped under, while every
//! object sealed under an older one still opens.

use tessari_s3_daemons::{Daemon, Rewrapper};
use tessari_s3_storage::keys::KeyState;
use tessari_s3_storage::objects::Content;
use tessari_s3_types::{BucketName, ObjectKey};

use crate::complete::{complete, document};
use crate::multipart::{etag, part, text};
use crate::sse::{SSE, body, passphrase, unseal};
use crate::{IGNORED, call, call_with, fresh_with, scratch_dir};

/// The id of the key `bucket/key`'s data key is wrapped under.
pub(crate) async fn wrapped_under(
    state: &tessari_s3_api::ApiState,
    bucket: &str,
    key: &str,
) -> String {
    state
        .storage()
        .objects()
        .get(
            &BucketName::new(bucket).expect("bucket"),
            &ObjectKey::new(key).expect("key"),
        )
        .await
        .expect("read")
        .ok()
        .flatten()
        .expect("the object")
        .encryption
        .expect("sealed")
        .kek_id
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_rotation_wraps_new_data_keys_under_a_new_server_key_and_old_objects_still_open() {
    let _why = IGNORED;
    let (state, _, dir) = fresh_with(scratch_dir()).await;
    unseal(&state).await;
    assert_eq!(call(&state, "PUT", "/keys", vec![], b"").await.status, 200);
    let bytes = body();
    let old = call_with(&state, "PUT", "/keys/old", vec![], vec![SSE], &bytes).await;
    assert_eq!(old.status, 200, "{}", old.body);
    assert_eq!(wrapped_under(&state, "keys", "old").await, "server_1");

    let rotated = state
        .storage()
        .keys()
        .rotate(&passphrase())
        .await
        .expect("rotated");
    let KeyState::Unsealed { active, keys, .. } = rotated else {
        panic!("still unsealed after a rotation: {rotated:?}");
    };
    assert_eq!((active.as_str(), keys), ("server_2", 2));

    let new = call_with(&state, "PUT", "/keys/new", vec![], vec![SSE], &bytes).await;
    assert_eq!(new.status, 200, "{}", new.body);
    assert_eq!(wrapped_under(&state, "keys", "new").await, "server_2");
    let reread = call(&state, "GET", "/keys/old", vec![], b"").await;
    assert!(
        reread.bytes == bytes,
        "an object under the old key still opens"
    );
    std::fs::remove_dir_all(dir).expect("cleanup");
}

/// Every file under `dir` with its bytes.
fn drive(dir: &std::path::Path) -> std::collections::BTreeMap<std::path::PathBuf, Vec<u8>> {
    let mut found = std::collections::BTreeMap::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(at) = pending.pop() {
        for entry in std::fs::read_dir(&at).expect("a readable directory") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let bytes = std::fs::read(&path).expect("a readable file");
                found.insert(path, bytes);
            }
        }
    }
    found
}

/// The key ids every part of the multipart object `bucket/key` is wrapped under.
async fn parts_under(state: &tessari_s3_api::ApiState, bucket: &str, key: &str) -> Vec<String> {
    let object = state
        .storage()
        .objects()
        .get(
            &BucketName::new(bucket).expect("bucket"),
            &ObjectKey::new(key).expect("key"),
        )
        .await
        .expect("read")
        .ok()
        .flatten()
        .expect("the object");
    let Content::Parts(multipart) = object.content else {
        panic!("not a multipart object");
    };
    multipart
        .parts
        .into_iter()
        .map(|part| part.encryption.expect("sealed").kek_id)
        .collect()
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn the_sweep_rewraps_every_data_key_under_the_new_key_and_leaves_every_byte_on_the_drive() {
    let _why = IGNORED;
    let (state, _, dir) = fresh_with(scratch_dir()).await;
    unseal(&state).await;
    assert_eq!(call(&state, "PUT", "/swept", vec![], b"").await.status, 200);
    let bytes = body();
    let one = call_with(&state, "PUT", "/swept/one", vec![], vec![SSE], &bytes).await;
    assert_eq!(one.status, 200, "{}", one.body);
    let created = call_with(
        &state,
        "POST",
        "/swept/many",
        vec![("uploads", None)],
        vec![SSE],
        b"",
    )
    .await;
    assert_eq!(created.status, 200, "{}", created.body);
    let upload = text(&created.body, "UploadId").expect("an upload id");
    let first = part(&state, "/swept/many", &upload, "1", &bytes).await;
    assert_eq!(first.status, 200, "{}", first.body);
    let done = complete(
        &state,
        "/swept/many",
        &upload,
        Vec::new(),
        &document(&[("1", etag(&first).as_str())]),
    )
    .await;
    assert_eq!(done.status, 200, "{}", done.body);
    let objects = state.storage().objects();
    assert!(objects.remaining("server_1").await.expect("counted") >= 2);
    let before = drive(&dir);
    assert!(before.len() >= 2, "control: the scan sees the data files");

    state
        .storage()
        .keys()
        .rotate(&passphrase())
        .await
        .expect("rotated");
    let rewrapper = Rewrapper::new(objects.clone());
    let report = rewrapper.run_once().await.expect("a pass");
    assert!(
        report.message.starts_with("re-wrapped "),
        "{}",
        report.message
    );
    assert_eq!(
        objects.remaining("server_1").await.expect("counted"),
        0,
        "nothing is left under the old key"
    );
    assert_eq!(wrapped_under(&state, "swept", "one").await, "server_2");
    assert_eq!(parts_under(&state, "swept", "many").await, ["server_2"]);
    assert!(drive(&dir) == before, "no data file changed");
    for key in ["one", "many"] {
        let read = call(&state, "GET", &format!("/swept/{key}"), vec![], b"").await;
        assert!(read.bytes == bytes, "{key} opens under its re-wrapped key");
    }
    let again = rewrapper.run_once().await.expect("a pass");
    assert_eq!(
        again.message, "re-wrapped 0 of 0",
        "a second pass finds nothing"
    );
    std::fs::remove_dir_all(dir).expect("cleanup");
}
