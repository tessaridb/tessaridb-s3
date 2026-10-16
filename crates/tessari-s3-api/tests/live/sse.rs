//! Server-side encryption with the server's own keys against a real node (G017 W3b): `x-amz-server-side-encryption:
//! AES256` seals an object's bytes on the drive and every read opens them, the header comes back on PUT, GET and HEAD,
//! and a node whose keys are sealed refuses to seal or open — while plain objects and HEAD go on working.

use tessari_s3_storage::keys::Passphrase;
use tessari_s3_storage::objects::Content;
use tessari_s3_types::{BucketName, ObjectKey};
use zeroize::Zeroizing;

use crate::complete::{complete, document};
use crate::multipart::{etag, part, text};
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

/// Whether data file `id` under `dir` contains `needle`.
fn file_holds(dir: &std::path::Path, id: [u8; 16], needle: &[u8]) -> bool {
    let name: String = id.iter().map(|byte| format!("{byte:02x}")).collect();
    let path = dir
        .join("s3data")
        .join(&name[0..2])
        .join(&name[2..4])
        .join(&name);
    let bytes = std::fs::read(path).expect("the data file");
    bytes.windows(needle.len()).any(|window| window == needle)
}

/// The data files holding `bucket/key`: its one file, or its parts' files in order.
async fn data_files(state: &tessari_s3_api::ApiState, bucket: &str, key: &str) -> Vec<[u8; 16]> {
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
    match object.content {
        Content::Data(id) => vec![id],
        Content::Parts(multipart) => multipart.parts.iter().map(|part| part.data).collect(),
    }
}

/// Whether any data file holding `bucket/key` contains `needle`.
async fn drive_holds(
    state: &tessari_s3_api::ApiState,
    dir: &std::path::Path,
    bucket: &str,
    key: &str,
    needle: &[u8],
) -> bool {
    data_files(state, bucket, key)
        .await
        .into_iter()
        .any(|id| file_holds(dir, id, needle))
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

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn multipart_parts_and_copies_asked_to_be_encrypted_are_sealed_and_read_whole() {
    let _why = IGNORED;
    let (state, _, dir) = fresh_with(scratch_dir()).await;
    assert_eq!(
        call(&state, "PUT", "/mpsealed", vec![], b"").await.status,
        200
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
    let bytes = body();
    let plain = call(&state, "PUT", "/mpsealed/plain", vec![], &bytes).await;
    assert_eq!(plain.status, 200, "{}", plain.body);

    // A multipart upload: one uploaded part past a block, then the plain object copied whole as the last part.
    let created = call_with(
        &state,
        "POST",
        "/mpsealed/m",
        vec![("uploads", None)],
        vec![SSE],
        b"",
    )
    .await;
    assert_eq!(created.status, 200, "{}", created.body);
    assert_eq!(sse_header(&created), Some(&b"AES256"[..]));
    let upload = text(&created.body, "UploadId").expect("an upload id");
    let first = vec![7_u8; 5 << 20];
    let one = part(&state, "/mpsealed/m", &upload, "1", &first).await;
    assert_eq!(one.status, 200, "{}", one.body);
    assert_eq!(sse_header(&one), Some(&b"AES256"[..]));
    let copied = call_with(
        &state,
        "PUT",
        "/mpsealed/m",
        vec![("partNumber", Some("2")), ("uploadId", Some(&upload))],
        vec![("x-amz-copy-source", "/mpsealed/plain")],
        b"",
    )
    .await;
    assert_eq!(copied.status, 200, "{}", copied.body);
    let two = text(&copied.body, "ETag").expect("an etag");
    let done = complete(
        &state,
        "/mpsealed/m",
        &upload,
        Vec::new(),
        &document(&[("1", etag(&one).as_str()), ("2", two.as_str())]),
    )
    .await;
    assert_eq!(done.status, 200, "{}", done.body);
    assert_eq!(sse_header(&done), Some(&b"AES256"[..]));
    let whole = [first.as_slice(), &bytes].concat();
    let got = call(&state, "GET", "/mpsealed/m", vec![], b"").await;
    assert_eq!(got.status, 200, "{}", got.body);
    assert_eq!(got.bytes, whole, "the parts read back in order");
    assert_eq!(sse_header(&got), Some(&b"AES256"[..]));
    assert_eq!(
        sse_header(&call(&state, "HEAD", "/mpsealed/m", vec![], b"").await),
        Some(&b"AES256"[..])
    );
    assert!(
        !drive_holds(&state, &dir, "mpsealed", "m", &bytes[..32]).await,
        "a copied part in clear"
    );
    assert!(
        !drive_holds(&state, &dir, "mpsealed", "m", &first[..32]).await,
        "an uploaded part in clear"
    );

    // A copy sealed from a plain source, and a plain copy of a sealed source.
    let sealed = call_with(
        &state,
        "PUT",
        "/mpsealed/sealed-copy",
        vec![],
        vec![SSE, ("x-amz-copy-source", "/mpsealed/plain")],
        b"",
    )
    .await;
    assert_eq!(sealed.status, 200, "{}", sealed.body);
    assert_eq!(sse_header(&sealed), Some(&b"AES256"[..]));
    assert!(!drive_holds(&state, &dir, "mpsealed", "sealed-copy", &bytes[..32]).await);
    assert_eq!(
        call(&state, "GET", "/mpsealed/sealed-copy", vec![], b"")
            .await
            .bytes,
        bytes
    );
    let unsealed = call_with(
        &state,
        "PUT",
        "/mpsealed/plain-copy",
        vec![],
        vec![("x-amz-copy-source", "/mpsealed/sealed-copy")],
        b"",
    )
    .await;
    assert_eq!(unsealed.status, 200, "{}", unsealed.body);
    assert_eq!(sse_header(&unsealed), None);
    assert!(
        drive_holds(&state, &dir, "mpsealed", "plain-copy", &bytes[..32]).await,
        "control: plain copy"
    );
    std::fs::remove_dir_all(dir).expect("cleanup");
}
