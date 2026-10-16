//! Server-side encryption with a customer-provided key against a real node (G017 W5): an object is sealed under the
//! key a request brings, only over TLS and only into a bucket that allows customer keys; every read must bring the
//! same key, the server keeps none of it, and the answers carry the key's MD5 and never the plaintext's.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use md5::{Digest, Md5};

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
