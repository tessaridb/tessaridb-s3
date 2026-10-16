//! Server-side encryption with the server's own keys against a real node (G017 W3b): `x-amz-server-side-encryption:
//! AES256` seals an object's bytes on the drive and every read opens them, the header comes back on PUT, GET and HEAD,
//! and a node whose keys are sealed refuses to seal or open — while plain objects and HEAD go on working.

use tessari_s3_storage::keys::Passphrase;
use tessari_s3_storage::objects::Content;
use tessari_s3_types::{BucketName, ObjectKey};
use zeroize::Zeroizing;

use crate::object_lock::refused;
use crate::{IGNORED, Seen, call, call_with, fresh_with, scratch_dir};

const SSE: (&str, &str) = ("x-amz-server-side-encryption", "AES256");

fn sse_header(seen: &Seen) -> Option<&[u8]> {
    seen.headers
        .get("x-amz-server-side-encryption")
        .map(|value| value.as_bytes())
}

/// Bytes past two whole blocks, a canary repeated so a raw scan can look for it.
fn body() -> Vec<u8> {
    b"a canary no drive may hold in clear "
        .iter()
        .copied()
        .cycle()
        .take((5 << 20) / 2)
        .collect()
}

/// Whether the data file holding `bucket/key` contains `needle`.
async fn drive_holds(
    state: &tessari_s3_api::ApiState,
    dir: &std::path::Path,
    bucket: &str,
    key: &str,
    needle: &[u8],
) -> bool {
    let bucket = BucketName::new(bucket).expect("bucket");
    let key = ObjectKey::new(key).expect("key");
    let object = state
        .storage()
        .objects()
        .get(&bucket, &key)
        .await
        .expect("read")
        .ok()
        .flatten()
        .expect("stored");
    let Content::Data(id) = object.content else {
        panic!("a single PUT has one data file")
    };
    let name: String = id.iter().map(|byte| format!("{byte:02x}")).collect();
    let path = dir
        .join("s3data")
        .join(&name[0..2])
        .join(&name[2..4])
        .join(&name);
    let bytes = std::fs::read(path).expect("the data file");
    bytes.windows(needle.len()).any(|window| window == needle)
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn an_object_asked_to_be_encrypted_is_sealed_on_the_drive_and_opened_on_every_read() {
    let _why = IGNORED;
    let (state, _, dir) = fresh_with(scratch_dir()).await;
    assert_eq!(
        call(&state, "PUT", "/sealed", vec![], b"").await.status,
        200
    );
    let bytes = body();

    // Keys sealed: nothing can be sealed, and the request says so rather than storing the bytes plain.
    let early = call_with(&state, "PUT", "/sealed/k", vec![], vec![SSE], &bytes).await;
    assert_eq!(
        refused(&early),
        (503, Some("ServiceUnavailable")),
        "{}",
        early.body
    );

    let passphrase = Passphrase::new(Zeroizing::new(
        "a keys passphrase for the sse suite".to_owned(),
    ))
    .expect("usable");
    state
        .storage()
        .keys()
        .unseal(&passphrase)
        .await
        .expect("unsealed");

    let put = call_with(&state, "PUT", "/sealed/k", vec![], vec![SSE], &bytes).await;
    assert_eq!(put.status, 200, "{}", put.body);
    assert_eq!(sse_header(&put), Some(&b"AES256"[..]));
    let plain = call(&state, "PUT", "/sealed/plain", vec![], &bytes).await;
    assert_eq!(plain.status, 200, "{}", plain.body);
    assert_eq!(sse_header(&plain), None);

    let got = call(&state, "GET", "/sealed/k", vec![], b"").await;
    assert_eq!(got.status, 200, "{}", got.body);
    assert_eq!(got.bytes, bytes, "the client reads back what it sent");
    assert_eq!(sse_header(&got), Some(&b"AES256"[..]));
    assert_eq!(
        got.headers.get("etag"),
        plain.headers.get("etag"),
        "the ETag is the MD5 of the plaintext under SSE-S3"
    );
    assert_eq!(
        sse_header(&call(&state, "HEAD", "/sealed/k", vec![], b"").await),
        Some(&b"AES256"[..])
    );
    // A range across the first block boundary.
    let from = (1_usize << 20) - 7;
    let range = format!("bytes={from}-{}", from + 20);
    let ranged = call_with(
        &state,
        "GET",
        "/sealed/k",
        vec![],
        vec![("range", range.as_str())],
        b"",
    )
    .await;
    assert_eq!(ranged.status, 206, "{}", ranged.body);
    assert_eq!(ranged.bytes, bytes[from..=from + 20]);

    // On the drive: the sealed object holds no plaintext; the plain one does, so the scan can see it.
    let needle = &bytes[..32];
    assert!(
        !drive_holds(&state, &dir, "sealed", "k", needle).await,
        "plaintext on the drive"
    );
    assert!(
        drive_holds(&state, &dir, "sealed", "plain", needle).await,
        "the scan finds plain bytes"
    );

    // Paths that cannot seal yet refuse rather than store plain.
    let multipart = call_with(
        &state,
        "POST",
        "/sealed/m",
        vec![("uploads", None)],
        vec![SSE],
        b"",
    )
    .await;
    assert_eq!(refused(&multipart), (501, Some("NotImplemented")));
    let copy = call_with(
        &state,
        "PUT",
        "/sealed/copy",
        vec![],
        vec![SSE, ("x-amz-copy-source", "/sealed/plain")],
        b"",
    )
    .await;
    assert_eq!(refused(&copy), (501, Some("NotImplemented")));

    // Keys sealed again: the sealed object cannot be read, the plain one and a HEAD still can.
    state.storage().keys().seal();
    let closed = call(&state, "GET", "/sealed/k", vec![], b"").await;
    assert_eq!(
        refused(&closed),
        (503, Some("ServiceUnavailable")),
        "{}",
        closed.body
    );
    assert_eq!(
        call(&state, "GET", "/sealed/plain", vec![], b"")
            .await
            .bytes,
        bytes
    );
    assert_eq!(
        call(&state, "HEAD", "/sealed/k", vec![], b"").await.status,
        200
    );
    std::fs::remove_dir_all(dir).expect("cleanup");
}
