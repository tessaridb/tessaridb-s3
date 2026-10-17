//! Bucket default encryption (G017 W4b): GetBucketEncryption, PutBucketEncryption and DeleteBucketEncryption, and the
//! default sealing every write that asks for no encryption — PUT, copy and multipart — while a write that asks for
//! its own keeps it. A default is accepted only when this node can seal under it.

use tessari_s3_storage::encryption::KeyName;
use tessari_s3_storage::keys::Passphrase;
use zeroize::Zeroizing;

use crate::multipart::text;
use crate::object_lock::refused;
use crate::sse::{SSE, body, drive_holds, sse_header, unseal};
use crate::{IGNORED, Seen, call, call_with, fresh_with, scratch_dir};

const KEY_ID: &str = "x-amz-server-side-encryption-aws-kms-key-id";

fn configuration(by_default: &str, rest: &str) -> Vec<u8> {
    format!(
        "<ServerSideEncryptionConfiguration><Rule><ApplyServerSideEncryptionByDefault>{by_default}\
         </ApplyServerSideEncryptionByDefault>{rest}</Rule></ServerSideEncryptionConfiguration>"
    )
    .into_bytes()
}

async fn put_default(state: &tessari_s3_api::ApiState, bucket: &str, document: &[u8]) -> Seen {
    call(
        state,
        "PUT",
        &format!("/{bucket}"),
        vec![("encryption", None)],
        document,
    )
    .await
}

async fn get_default(state: &tessari_s3_api::ApiState, bucket: &str) -> Seen {
    call(
        state,
        "GET",
        &format!("/{bucket}"),
        vec![("encryption", None)],
        b"",
    )
    .await
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_bucket_default_seals_every_write_that_asks_for_no_encryption() {
    let _why = IGNORED;
    let (state, _, dir) = fresh_with(scratch_dir()).await;
    for bucket in ["/defaulted", "/source"] {
        assert_eq!(call(&state, "PUT", bucket, vec![], b"").await.status, 200);
    }
    let aes = configuration("<SSEAlgorithm>AES256</SSEAlgorithm>", "");
    let none = get_default(&state, "defaulted").await;
    assert_eq!(
        refused(&none),
        (400, Some("ServerSideEncryptionConfigurationNotFoundError")),
        "{}",
        none.body
    );
    let early = put_default(&state, "defaulted", &aes).await;
    assert_eq!(
        refused(&early),
        (503, Some("ServiceUnavailable")),
        "a node that cannot seal accepts no default: {}",
        early.body
    );

    unseal(&state).await;
    let set = put_default(&state, "defaulted", &aes).await;
    assert_eq!(set.status, 200, "{}", set.body);
    let answered = get_default(&state, "defaulted").await;
    assert_eq!(answered.status, 200, "{}", answered.body);
    assert_eq!(
        text(&answered.body, "SSEAlgorithm").as_deref(),
        Some("AES256")
    );

    let bytes = body();
    let needle = &bytes[..32];
    let put = call(&state, "PUT", "/defaulted/put", vec![], &bytes).await;
    assert_eq!(put.status, 200, "{}", put.body);
    assert_eq!(
        sse_header(&put),
        Some(&b"AES256"[..]),
        "the default answers"
    );
    assert!(
        !drive_holds(&state, &dir, "defaulted", "put", needle).await,
        "a write under the default is plaintext on the drive"
    );
    let got = call(&state, "GET", "/defaulted/put", vec![], b"").await;
    assert_eq!(got.bytes, bytes);
    assert_eq!(sse_header(&got), Some(&b"AES256"[..]));

    let plain = call(&state, "PUT", "/source/plain", vec![], &bytes).await;
    assert_eq!(
        sse_header(&plain),
        None,
        "control: a bucket with no default"
    );
    let copied = call_with(
        &state,
        "PUT",
        "/defaulted/copied",
        vec![],
        vec![("x-amz-copy-source", "/source/plain")],
        b"",
    )
    .await;
    assert_eq!(copied.status, 200, "{}", copied.body);
    assert_eq!(sse_header(&copied), Some(&b"AES256"[..]));
    assert!(!drive_holds(&state, &dir, "defaulted", "copied", needle).await);

    let created = call(
        &state,
        "POST",
        "/defaulted/multi",
        vec![("uploads", None)],
        b"",
    )
    .await;
    assert_eq!(created.status, 200, "{}", created.body);
    assert_eq!(
        sse_header(&created),
        Some(&b"AES256"[..]),
        "an upload takes the default at its creation"
    );

    let removed = call(
        &state,
        "DELETE",
        "/defaulted",
        vec![("encryption", None)],
        b"",
    )
    .await;
    assert_eq!(removed.status, 204, "{}", removed.body);
    assert_eq!(
        refused(&get_default(&state, "defaulted").await).1,
        Some("ServerSideEncryptionConfigurationNotFoundError")
    );
    let after = call(&state, "PUT", "/defaulted/after", vec![], &bytes).await;
    assert_eq!(sse_header(&after), None, "no default, no sealing");
    assert!(
        drive_holds(&state, &dir, "defaulted", "after", needle).await,
        "control: the scan finds plain bytes"
    );
    std::fs::remove_dir_all(dir).expect("cleanup");
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_named_key_default_needs_the_key_and_yields_to_a_header() {
    let _why = IGNORED;
    let (state, _, dir) = fresh_with(scratch_dir()).await;
    assert_eq!(call(&state, "PUT", "/keyed", vec![], b"").await.status, 200);
    unseal(&state).await;
    let kms = |id: &str| {
        configuration(
            &format!("<SSEAlgorithm>aws:kms</SSEAlgorithm><KMSMasterKeyID>{id}</KMSMasterKeyID>"),
            "<BucketKeyEnabled>true</BucketKeyEnabled>",
        )
    };
    let missing = put_default(&state, "keyed", &kms("alias/bucket-7")).await;
    assert_eq!(
        refused(&missing),
        (400, Some("KMS.NotFoundException")),
        "{}",
        missing.body
    );

    let passphrase = Passphrase::new(Zeroizing::new(
        "a keys passphrase for the sse suite".to_owned(),
    ))
    .expect("usable");
    state
        .storage()
        .keys()
        .create_named(&passphrase, &KeyName::new("bucket-7").expect("usable"))
        .await
        .expect("created");
    let set = put_default(&state, "keyed", &kms("alias/bucket-7")).await;
    assert_eq!(set.status, 200, "{}", set.body);
    let answered = get_default(&state, "keyed").await;
    assert_eq!(
        text(&answered.body, "KMSMasterKeyID").as_deref(),
        Some("bucket-7")
    );
    assert_eq!(
        text(&answered.body, "BucketKeyEnabled").as_deref(),
        Some("true")
    );

    let bytes = body();
    let put = call(&state, "PUT", "/keyed/k", vec![], &bytes).await;
    assert_eq!(put.status, 200, "{}", put.body);
    assert_eq!(sse_header(&put), Some(&b"aws:kms"[..]));
    assert_eq!(
        put.headers
            .get(KEY_ID)
            .and_then(|value| value.to_str().ok()),
        Some("bucket-7")
    );
    assert!(!drive_holds(&state, &dir, "keyed", "k", &bytes[..32]).await);

    let asked = call_with(&state, "PUT", "/keyed/asked", vec![], vec![SSE], &bytes).await;
    assert_eq!(
        sse_header(&asked),
        Some(&b"AES256"[..]),
        "a write that asks for its own encryption keeps it"
    );
    assert_eq!(asked.headers.get(KEY_ID), None);
    std::fs::remove_dir_all(dir).expect("cleanup");
}
