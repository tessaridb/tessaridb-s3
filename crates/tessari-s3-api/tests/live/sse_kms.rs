//! Server-side encryption under keys an operator creates by name (`aws:kms`, G017 W4): a named key seals PUTs,
//! multipart uploads and copies; the response names the key; the ETag is no MD5 of the plaintext; a key nobody
//! created is refused as S3 refuses it; and the key outlives the node's ring, read back from the keys vault.

use tessari_s3_core::objects::checksum::Hashes;
use tessari_s3_storage::encryption::KeyName;
use tessari_s3_storage::keys::Passphrase;
use tessari_s3_types::{BucketName, ObjectKey};
use zeroize::Zeroizing;

use crate::complete::{complete, document};
use crate::multipart::{etag, part, text};
use crate::object_lock::refused;
use crate::sse::{body, drive_holds, unseal};
use crate::{IGNORED, Seen, call, call_with, fresh_with, scratch_dir};

const MODE: &str = "x-amz-server-side-encryption";
const KEY_ID: &str = "x-amz-server-side-encryption-aws-kms-key-id";

fn header<'a>(seen: &'a Seen, name: &str) -> Option<&'a str> {
    seen.headers.get(name).and_then(|value| value.to_str().ok())
}

/// The mode and the key id a response answers.
fn answered(seen: &Seen) -> (Option<&str>, Option<&str>) {
    (header(seen, MODE), header(seen, KEY_ID))
}

/// The id of the key that wrapped `bucket/key`'s data key.
async fn kek_of(state: &tessari_s3_api::ApiState, bucket: &str, key: &str) -> String {
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
        .expect("stored")
        .encryption
        .expect("sealed")
        .kek_id
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn objects_sealed_under_a_named_key_name_it_and_carry_no_plaintext_md5() {
    let _why = IGNORED;
    let (state, _, dir) = fresh_with(scratch_dir()).await;
    assert_eq!(
        call(&state, "PUT", "/vaulted", vec![], b"").await.status,
        200
    );
    unseal(&state).await;
    let bytes = body();
    let kms = || vec![(MODE, "aws:kms"), (KEY_ID, "alias/tenant-7")];

    // No such key yet, named or the default one.
    for headers in [kms(), vec![(MODE, "aws:kms")]] {
        let early = call_with(&state, "PUT", "/vaulted/k", vec![], headers, &bytes).await;
        assert_eq!(
            refused(&early),
            (400, Some("KMS.NotFoundException")),
            "{}",
            early.body
        );
    }

    let passphrase = Passphrase::new(Zeroizing::new(
        "a keys passphrase for the sse suite".to_owned(),
    ))
    .expect("usable");
    let name = KeyName::new("tenant-7").expect("usable");
    state
        .storage()
        .keys()
        .create_named(&passphrase, &name)
        .await
        .expect("created");
    assert!(
        state
            .storage()
            .keys()
            .create_named(&passphrase, &name)
            .await
            .is_err(),
        "a key is never replaced"
    );

    let put = call_with(&state, "PUT", "/vaulted/k", vec![], kms(), &bytes).await;
    assert_eq!(put.status, 200, "{}", put.body);
    assert_eq!(answered(&put), (Some("aws:kms"), Some("tenant-7")));
    assert_eq!(kek_of(&state, "vaulted", "k").await, "kms_tenant-7");
    let plain = call(&state, "PUT", "/vaulted/plain", vec![], &bytes).await;
    let again = call_with(&state, "PUT", "/vaulted/again", vec![], kms(), &bytes).await;
    assert_ne!(
        header(&put, "etag"),
        header(&plain, "etag"),
        "no MD5 of the plaintext under a named key"
    );
    assert_ne!(
        header(&put, "etag"),
        header(&again, "etag"),
        "the same bytes twice do not share an ETag"
    );
    let got = call(&state, "GET", "/vaulted/k", vec![], b"").await;
    assert_eq!(got.bytes, bytes);
    assert_eq!(answered(&got), (Some("aws:kms"), Some("tenant-7")));
    assert_eq!(header(&got, "etag"), header(&put, "etag"));
    assert!(!drive_holds(&state, &dir, "vaulted", "k", &bytes[..32]).await);
    assert!(
        drive_holds(&state, &dir, "vaulted", "plain", &bytes[..32]).await,
        "control: the scan finds plain bytes"
    );

    // Multipart: the upload asks once, every part is sealed under the key, and so is a part copied in.
    let created = call_with(
        &state,
        "POST",
        "/vaulted/m",
        vec![("uploads", None)],
        kms(),
        b"",
    )
    .await;
    assert_eq!(answered(&created), (Some("aws:kms"), Some("tenant-7")));
    let upload = text(&created.body, "UploadId").expect("an upload id");
    let first = vec![7_u8; 5 << 20];
    let one = part(&state, "/vaulted/m", &upload, "1", &first).await;
    assert_eq!(one.status, 200, "{}", one.body);
    assert_eq!(answered(&one), (Some("aws:kms"), Some("tenant-7")));
    let mut hashes = Hashes::new();
    hashes.update(&first);
    let plain_part = format!("\"{}\"", hashes.finish().md5_hex);
    assert_ne!(etag(&one), plain_part, "a part's ETag is no MD5 either");
    let copied = call_with(
        &state,
        "PUT",
        "/vaulted/m",
        vec![("partNumber", Some("2")), ("uploadId", Some(&upload))],
        vec![("x-amz-copy-source", "/vaulted/plain")],
        b"",
    )
    .await;
    assert_eq!(copied.status, 200, "{}", copied.body);
    let two = text(&copied.body, "ETag").expect("an etag");
    let done = complete(
        &state,
        "/vaulted/m",
        &upload,
        Vec::new(),
        &document(&[("1", etag(&one).as_str()), ("2", two.as_str())]),
    )
    .await;
    assert_eq!(done.status, 200, "{}", done.body);
    assert_eq!(answered(&done), (Some("aws:kms"), Some("tenant-7")));
    let whole = call(&state, "GET", "/vaulted/m", vec![], b"").await;
    assert_eq!(whole.bytes, [first.as_slice(), &bytes].concat());
    assert_eq!(answered(&whole), (Some("aws:kms"), Some("tenant-7")));
    assert!(!drive_holds(&state, &dir, "vaulted", "m", &bytes[..32]).await);

    // A copy asked to be sealed under the key.
    let copy = call_with(
        &state,
        "PUT",
        "/vaulted/copy",
        vec![],
        vec![
            (MODE, "aws:kms"),
            (KEY_ID, "tenant-7"),
            ("x-amz-copy-source", "/vaulted/plain"),
        ],
        b"",
    )
    .await;
    assert_eq!(copy.status, 200, "{}", copy.body);
    assert_eq!(answered(&copy), (Some("aws:kms"), Some("tenant-7")));
    assert_eq!(kek_of(&state, "vaulted", "copy").await, "kms_tenant-7");

    // The key is in the vault, not only in this node's memory: sealed and unsealed again, it still opens.
    state.storage().keys().seal();
    unseal(&state).await;
    assert_eq!(
        call(&state, "GET", "/vaulted/k", vec![], b"").await.bytes,
        bytes
    );
    let unknown = call_with(
        &state,
        "PUT",
        "/vaulted/x",
        vec![],
        vec![(MODE, "aws:kms"), (KEY_ID, "nobody")],
        b"x",
    )
    .await;
    assert_eq!(refused(&unknown), (400, Some("KMS.NotFoundException")));

    // No key id names S3's own managed key, `aws-s3`, once an operator has created it.
    let managed = KeyName::new("aws-s3").expect("usable");
    state
        .storage()
        .keys()
        .create_named(&passphrase, &managed)
        .await
        .expect("created");
    let defaulted = call_with(
        &state,
        "PUT",
        "/vaulted/default",
        vec![],
        vec![(MODE, "aws:kms")],
        &bytes,
    )
    .await;
    assert_eq!(defaulted.status, 200, "{}", defaulted.body);
    assert_eq!(answered(&defaulted), (Some("aws:kms"), Some("aws-s3")));
    assert_eq!(kek_of(&state, "vaulted", "default").await, "kms_aws-s3");
    std::fs::remove_dir_all(dir).expect("cleanup");
}
