//! Server-side encryption with a customer-provided key against a real node (G017 W5): an object is sealed under the
//! key a request brings, only over TLS and only into a bucket that allows customer keys; every read must bring the
//! same key, the server keeps none of it, and the answers carry the key's MD5 and never the plaintext's.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use md5::{Digest, Md5};

use crate::multipart::text;
use crate::object_lock::refused;
use crate::sse::{body, drive_holds, sse_header, unseal};
use crate::{IGNORED, Seen, call, call_tls, call_with, fresh_with, scratch_dir};

const ALGORITHM: &str = "x-amz-server-side-encryption-customer-algorithm";
const KEY: &str = "x-amz-server-side-encryption-customer-key";
const KEY_MD5: &str = "x-amz-server-side-encryption-customer-key-md5";

/// The three headers that bring a customer's key.
struct Customer {
    key: String,
    md5: String,
}

impl Customer {
    fn of(key: &[u8]) -> Self {
        Self {
            key: STANDARD.encode(key),
            md5: STANDARD.encode(Md5::digest(key)),
        }
    }

    fn headers(&self) -> Vec<(&str, &str)> {
        vec![
            (ALGORITHM, "AES256"),
            (KEY, self.key.as_str()),
            (KEY_MD5, self.md5.as_str()),
        ]
    }
}

fn header<'a>(seen: &'a Seen, name: &str) -> Option<&'a str> {
    seen.headers.get(name).and_then(|value| value.to_str().ok())
}

/// A bucket `name` whose default encryption allows customer keys, on a node whose keys are unsealed.
async fn allowing(state: &tessari_s3_api::ApiState, name: &str) {
    unseal(state).await;
    let created = call(state, "PUT", &format!("/{name}"), vec![], b"").await;
    assert_eq!(created.status, 200, "{}", created.body);
    let document = b"<ServerSideEncryptionConfiguration><Rule><ApplyServerSideEncryptionByDefault>\
        <SSEAlgorithm>AES256</SSEAlgorithm></ApplyServerSideEncryptionByDefault><BlockedEncryptionTypes>\
        <EncryptionType>NONE</EncryptionType></BlockedEncryptionTypes></Rule></ServerSideEncryptionConfiguration>";
    let allowed = call(
        state,
        "PUT",
        &format!("/{name}"),
        vec![("encryption", None)],
        document,
    )
    .await;
    assert_eq!(allowed.status, 200, "{}", allowed.body);
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_customer_key_seals_an_object_only_over_tls_into_a_bucket_that_allows_it() {
    let _why = IGNORED;
    let (state, _, dir) = fresh_with(scratch_dir()).await;
    let customer = Customer::of(&[7_u8; 32]);
    let bytes = body();

    let blocked = call(&state, "PUT", "/blocked", vec![], b"").await;
    assert_eq!(blocked.status, 200, "{}", blocked.body);
    let into_blocked = call_tls(
        &state,
        "PUT",
        "/blocked/object",
        vec![],
        customer.headers(),
        &bytes,
    )
    .await;
    assert_eq!(
        refused(&into_blocked),
        (403, Some("AccessDenied")),
        "a bucket allows customer keys only when told to: {}",
        into_blocked.body
    );

    allowing(&state, "sealed").await;
    let in_clear = call_with(
        &state,
        "PUT",
        "/sealed/object",
        vec![],
        customer.headers(),
        &bytes,
    )
    .await;
    assert_eq!(
        refused(&in_clear),
        (400, Some("InvalidRequest")),
        "a key sent in the clear is refused: {}",
        in_clear.body
    );
    let nothing = call_tls(
        &state,
        "GET",
        "/sealed/object",
        vec![],
        customer.headers(),
        b"",
    )
    .await;
    assert_eq!(
        refused(&nothing),
        (404, Some("NoSuchKey")),
        "{}",
        nothing.body
    );

    let put = call_tls(
        &state,
        "PUT",
        "/sealed/object",
        vec![],
        customer.headers(),
        &bytes,
    )
    .await;
    assert_eq!(put.status, 200, "{}", put.body);
    assert_eq!(header(&put, ALGORITHM), Some("AES256"));
    assert_eq!(header(&put, KEY_MD5), Some(customer.md5.as_str()));
    assert_eq!(header(&put, KEY), None, "the key itself is never answered");
    assert_eq!(sse_header(&put), None, "a customer key is not the server's");
    let plaintext_md5: String = Md5::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_ne!(
        header(&put, "etag"),
        Some(format!("\"{plaintext_md5}\"").as_str()),
        "the ETag does not tell which sealed objects hold the same bytes"
    );
    assert!(
        !drive_holds(&state, &dir, "sealed", "object", &bytes[..64]).await,
        "the drive holds no plaintext"
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn reading_a_customer_sealed_object_needs_the_key_that_sealed_it() {
    let _why = IGNORED;
    let (state, _, _dir) = fresh_with(scratch_dir()).await;
    allowing(&state, "sealed").await;
    let customer = Customer::of(&[7_u8; 32]);
    let other = Customer::of(&[8_u8; 32]);
    let bytes = body();
    let put = call_tls(
        &state,
        "PUT",
        "/sealed/object",
        vec![],
        customer.headers(),
        &bytes,
    )
    .await;
    assert_eq!(put.status, 200, "{}", put.body);

    let keyless = call_tls(&state, "GET", "/sealed/object", vec![], vec![], b"").await;
    assert_eq!(
        refused(&keyless),
        (400, Some("InvalidRequest")),
        "{}",
        keyless.body
    );
    let wrong = call_tls(
        &state,
        "GET",
        "/sealed/object",
        vec![],
        other.headers(),
        b"",
    )
    .await;
    assert_eq!(
        refused(&wrong),
        (403, Some("AccessDenied")),
        "{}",
        wrong.body
    );
    let read = call_tls(
        &state,
        "GET",
        "/sealed/object",
        vec![],
        customer.headers(),
        b"",
    )
    .await;
    assert_eq!(read.status, 200, "{}", read.body);
    assert!(read.bytes == bytes, "the key opens what it sealed");
    assert_eq!(header(&read, ALGORITHM), Some("AES256"));
    assert_eq!(header(&read, KEY_MD5), Some(customer.md5.as_str()));
    assert_eq!(header(&read, "etag"), header(&put, "etag"));

    let mut ranged = customer.headers();
    ranged.push(("range", "bytes=100-199"));
    let slice = call_tls(&state, "GET", "/sealed/object", vec![], ranged, b"").await;
    assert_eq!(slice.status, 206, "{}", slice.body);
    assert!(
        slice.bytes == bytes[100..200],
        "a range opens under the key too"
    );

    let head_keyless = call_tls(&state, "HEAD", "/sealed/object", vec![], vec![], b"").await;
    assert_eq!(head_keyless.status, 400);
    let head_wrong = call_tls(
        &state,
        "HEAD",
        "/sealed/object",
        vec![],
        other.headers(),
        b"",
    )
    .await;
    assert_eq!(head_wrong.status, 403);
    let head = call_tls(
        &state,
        "HEAD",
        "/sealed/object",
        vec![],
        customer.headers(),
        b"",
    )
    .await;
    assert_eq!(head.status, 200);
    assert_eq!(header(&head, KEY_MD5), Some(customer.md5.as_str()));

    let plain = call(&state, "PUT", "/sealed/plain", vec![], b"plain bytes").await;
    assert_eq!(plain.status, 200, "{}", plain.body);
    let keyed_plain = call_tls(
        &state,
        "GET",
        "/sealed/plain",
        vec![],
        customer.headers(),
        b"",
    )
    .await;
    assert_eq!(
        refused(&keyed_plain),
        (400, Some("InvalidRequest")),
        "a key for an object sealed without one is refused: {}",
        keyed_plain.body
    );

    let copy = call_with(
        &state,
        "PUT",
        "/sealed/copied",
        vec![],
        vec![("x-amz-copy-source", "/sealed/object")],
        b"",
    )
    .await;
    assert_eq!(
        refused(&copy),
        (400, Some("InvalidRequest")),
        "a copy that brings no key for its sealed source is refused, not failed: {}",
        copy.body
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn customer_key_headers_are_checked_before_anything_is_stored() {
    let _why = IGNORED;
    let (state, _, _dir) = fresh_with(scratch_dir()).await;
    allowing(&state, "sealed").await;
    let customer = Customer::of(&[7_u8; 32]);
    let other = Customer::of(&[8_u8; 32]);

    let mismatched = vec![
        (ALGORITHM, "AES256"),
        (KEY, customer.key.as_str()),
        (KEY_MD5, other.md5.as_str()),
    ];
    let mut beside_server = customer.headers();
    beside_server.push(("x-amz-server-side-encryption", "AES256"));
    let mut wrong_algorithm = customer.headers();
    wrong_algorithm[0] = (ALGORITHM, "AES128");
    for (headers, expected) in [
        (mismatched, "InvalidArgument"),
        (beside_server, "InvalidArgument"),
        (wrong_algorithm, "InvalidEncryptionAlgorithmError"),
    ] {
        let put = call_tls(&state, "PUT", "/sealed/object", vec![], headers, b"bytes").await;
        assert_eq!(refused(&put), (400, Some(expected)), "{}", put.body);
    }
    let nothing = call_tls(
        &state,
        "GET",
        "/sealed/object",
        vec![],
        customer.headers(),
        b"",
    )
    .await;
    assert_eq!(
        refused(&nothing),
        (404, Some("NoSuchKey")),
        "{}",
        nothing.body
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn every_part_of_a_customer_sealed_upload_must_bring_the_key_it_was_created_with() {
    let _why = IGNORED;
    let (state, _, dir) = fresh_with(scratch_dir()).await;
    allowing(&state, "sealed").await;
    let customer = Customer::of(&[7_u8; 32]);
    let other = Customer::of(&[8_u8; 32]);
    let path = "/sealed/large";
    // Every part but the last holds at least 5 MiB.
    let first = [body(), body()].concat();
    let second = b"the last part may be small".to_vec();

    let in_clear = call_with(
        &state,
        "POST",
        path,
        vec![("uploads", None)],
        customer.headers(),
        b"",
    )
    .await;
    assert_eq!(
        refused(&in_clear),
        (400, Some("InvalidRequest")),
        "{}",
        in_clear.body
    );
    let created = call_tls(
        &state,
        "POST",
        path,
        vec![("uploads", None)],
        customer.headers(),
        b"",
    )
    .await;
    assert_eq!(created.status, 200, "{}", created.body);
    assert_eq!(header(&created, KEY_MD5), Some(customer.md5.as_str()));
    let upload = text(&created.body, "UploadId").expect("an upload id");
    let at = |number: &'static str| {
        vec![
            ("partNumber", Some(number)),
            ("uploadId", Some(upload.as_str())),
        ]
    };

    let keyless = call_tls(&state, "PUT", path, at("1"), vec![], &first).await;
    assert_eq!(
        refused(&keyless),
        (400, Some("InvalidRequest")),
        "{}",
        keyless.body
    );
    let wrong = call_tls(&state, "PUT", path, at("1"), other.headers(), &first).await;
    assert_eq!(
        refused(&wrong),
        (403, Some("AccessDenied")),
        "{}",
        wrong.body
    );
    let copied = call_tls(
        &state,
        "PUT",
        path,
        at("1"),
        vec![("x-amz-copy-source", "/sealed/large")],
        b"",
    )
    .await;
    assert_eq!(
        refused(&copied),
        (400, Some("InvalidRequest")),
        "a copied part brings the upload's key too: {}",
        copied.body
    );
    let one = call_tls(&state, "PUT", path, at("1"), customer.headers(), &first).await;
    assert_eq!(one.status, 200, "{}", one.body);
    assert_eq!(header(&one, KEY_MD5), Some(customer.md5.as_str()));
    assert_ne!(
        header(&one, "etag").map(str::to_owned),
        Some(format!("\"{}\"", crate::multipart::md5_hex(&first))),
        "a part's ETag does not tell its bytes either"
    );
    let two = call_tls(&state, "PUT", path, at("2"), customer.headers(), &second).await;
    assert_eq!(two.status, 200, "{}", two.body);

    let listed = crate::complete::document(&[
        ("1", header(&one, "etag").expect("an ETag")),
        ("2", header(&two, "etag").expect("an ETag")),
    ]);
    let done = call_tls(
        &state,
        "POST",
        path,
        vec![("uploadId", Some(upload.as_str()))],
        vec![],
        &listed,
    )
    .await;
    assert_eq!(done.status, 200, "completing needs no key: {}", done.body);

    let keyless_read = call_tls(&state, "GET", path, vec![], vec![], b"").await;
    assert_eq!(
        refused(&keyless_read),
        (400, Some("InvalidRequest")),
        "{}",
        keyless_read.body
    );
    let read = call_tls(&state, "GET", path, vec![], customer.headers(), b"").await;
    assert_eq!(read.status, 200, "{}", read.body);
    assert!(
        read.bytes == [first.as_slice(), second.as_slice()].concat(),
        "every part opens under the key"
    );
    let mut across = customer.headers();
    let end = first.len() + 4;
    let range = format!("bytes={}-{end}", first.len() - 5);
    across.push(("range", range.as_str()));
    let slice = call_tls(&state, "GET", path, vec![], across, b"").await;
    assert_eq!(slice.status, 206, "{}", slice.body);
    assert!(
        slice.bytes == [&first[first.len() - 5..], &second[..5]].concat(),
        "a range across parts opens both"
    );
    assert!(
        !drive_holds(&state, &dir, "sealed", "large", &first[..64]).await,
        "the drive holds no plaintext"
    );
}

/// The three headers that bring the key a copy's source is sealed under.
fn source_headers(customer: &Customer) -> Vec<(&str, &str)> {
    vec![
        (
            "x-amz-copy-source-server-side-encryption-customer-algorithm",
            "AES256",
        ),
        (
            "x-amz-copy-source-server-side-encryption-customer-key",
            customer.key.as_str(),
        ),
        (
            "x-amz-copy-source-server-side-encryption-customer-key-md5",
            customer.md5.as_str(),
        ),
    ]
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_copy_opens_its_source_and_seals_its_destination_each_under_the_key_it_names() {
    let _why = IGNORED;
    let (state, _, dir) = fresh_with(scratch_dir()).await;
    allowing(&state, "sealed").await;
    let first = Customer::of(&[7_u8; 32]);
    let second = Customer::of(&[8_u8; 32]);
    let bytes = body();
    let put = call_tls(&state, "PUT", "/sealed/a", vec![], first.headers(), &bytes).await;
    assert_eq!(put.status, 200, "{}", put.body);
    let copying = |source: &'static str| ("x-amz-copy-source", source);

    let mut wrong = source_headers(&second);
    wrong.push(copying("/sealed/a"));
    let refused_wrong = call_tls(&state, "PUT", "/sealed/b", vec![], wrong, b"").await;
    assert_eq!(refused_wrong.status, 403, "{}", refused_wrong.body);
    let mut in_clear = source_headers(&first);
    in_clear.push(copying("/sealed/a"));
    let refused_clear = call_with(&state, "PUT", "/sealed/b", vec![], in_clear, b"").await;
    assert_eq!(
        refused(&refused_clear),
        (400, Some("InvalidRequest")),
        "{}",
        refused_clear.body
    );

    let mut both = source_headers(&first);
    both.extend(second.headers());
    both.push(copying("/sealed/a"));
    let copied = call_tls(&state, "PUT", "/sealed/b", vec![], both, b"").await;
    assert_eq!(copied.status, 200, "{}", copied.body);
    assert_eq!(header(&copied, KEY_MD5), Some(second.md5.as_str()));
    assert_ne!(
        text(&copied.body, "ETag"),
        Some(format!("\"{}\"", crate::multipart::md5_hex(&bytes))),
        "a copy's ETag does not tell its bytes either"
    );
    let under_second = call_tls(&state, "GET", "/sealed/b", vec![], second.headers(), b"").await;
    assert!(
        under_second.bytes == bytes,
        "the destination opens under its own key"
    );
    let under_first = call_tls(&state, "GET", "/sealed/b", vec![], first.headers(), b"").await;
    assert_eq!(under_first.status, 403, "{}", under_first.body);
    assert!(
        !drive_holds(&state, &dir, "sealed", "b", &bytes[..64]).await,
        "the copy's drive holds no plaintext"
    );

    let mut rotate = source_headers(&first);
    rotate.extend(second.headers());
    rotate.push(copying("/sealed/a"));
    let rotated = call_tls(&state, "PUT", "/sealed/a", vec![], rotate, b"").await;
    assert_eq!(
        rotated.status, 200,
        "a copy onto itself changes its key: {}",
        rotated.body
    );
    let reread = call_tls(&state, "GET", "/sealed/a", vec![], second.headers(), b"").await;
    assert!(reread.bytes == bytes, "the object opens under its new key");

    let plain = call(&state, "PUT", "/sealed/plain", vec![], b"plain bytes").await;
    assert_eq!(plain.status, 200, "{}", plain.body);
    let mut sealing_plain = first.headers();
    sealing_plain.push(copying("/sealed/plain"));
    let sealed_copy = call_tls(&state, "PUT", "/sealed/c", vec![], sealing_plain, b"").await;
    assert_eq!(sealed_copy.status, 200, "{}", sealed_copy.body);
    let keyless = call_tls(&state, "GET", "/sealed/c", vec![], vec![], b"").await;
    assert_eq!(
        refused(&keyless),
        (400, Some("InvalidRequest")),
        "{}",
        keyless.body
    );

    let created = call_tls(
        &state,
        "POST",
        "/sealed/parts",
        vec![("uploads", None)],
        first.headers(),
        b"",
    )
    .await;
    assert_eq!(created.status, 200, "{}", created.body);
    let upload = text(&created.body, "UploadId").expect("an upload id");
    let at = vec![
        ("partNumber", Some("1")),
        ("uploadId", Some(upload.as_str())),
    ];
    let mut source_only = source_headers(&second);
    source_only.push(copying("/sealed/a"));
    let without_upload_key =
        call_tls(&state, "PUT", "/sealed/parts", at.clone(), source_only, b"").await;
    assert_eq!(
        refused(&without_upload_key),
        (400, Some("InvalidRequest")),
        "a part copied into the upload brings the upload's key too: {}",
        without_upload_key.body
    );
    let mut part_both = source_headers(&second);
    part_both.extend(first.headers());
    part_both.push(copying("/sealed/a"));
    let part = call_tls(&state, "PUT", "/sealed/parts", at, part_both, b"").await;
    assert_eq!(part.status, 200, "{}", part.body);
    let part_etag = text(&part.body, "ETag").expect("a part ETag");
    let listed = crate::complete::document(&[("1", part_etag.as_str())]);
    let done = call_tls(
        &state,
        "POST",
        "/sealed/parts",
        vec![("uploadId", Some(upload.as_str()))],
        vec![],
        &listed,
    )
    .await;
    assert_eq!(done.status, 200, "{}", done.body);
    let assembled = call_tls(&state, "GET", "/sealed/parts", vec![], first.headers(), b"").await;
    assert!(
        assembled.bytes == bytes,
        "a copied part opens under the upload's key"
    );
}
