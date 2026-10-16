//! Rotating the server key (G017 W6): a new server key becomes the one new data keys are wrapped under, while every
//! object sealed under an older one still opens.

use tessari_s3_storage::keys::KeyState;
use tessari_s3_types::{BucketName, ObjectKey};

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
