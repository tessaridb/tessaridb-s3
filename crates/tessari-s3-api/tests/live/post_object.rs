//! A browser's form upload against a real node (S3 ref 03 M7): `POST /{bucket}` with a signed policy, checked on the
//! expanded form with every field covered, the file streamed with its length taken from the request, and the upload
//! authorized as the signer's PutObject.

use axum::body::Body;
use axum::http::Request;
use base64::Engine as _;
use tessari_s3_core::auth::{SecretKey, post_signature};

use crate::policy::put_policy;
use crate::public_access::allow_public;
use crate::{ACCESS_KEY, IGNORED, SECRET, Seen, call, fresh, send};

/// Why, the form's fields, the file's name and bytes, how the body ends, and the refusal.
type Case = (
    &'static str,
    Vec<(String, String)>,
    &'static str,
    &'static [u8],
    String,
    &'static str,
);

const BOUNDARY: &str = "----form0boundary7";
pub(crate) const DATE: &str = "20261013T000000Z";

fn header<'a>(seen: &'a Seen, name: &str) -> Option<&'a str> {
    seen.headers.get(name).and_then(|value| value.to_str().ok())
}

/// The policy in its encoded form, its conditions as JSON list items.
pub(crate) fn policy(conditions: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(format!(
        r#"{{"expiration":"2099-01-01T00:00:00Z","conditions":[{conditions}]}}"#
    ))
}

/// The signing fields for `encoded` under the live credential, in `region`.
fn signed(state: &tessari_s3_api::ApiState, encoded: &str) -> Vec<(String, String)> {
    let region = state.region();
    let signature = post_signature(
        &SecretKey::new(SECRET.to_owned()),
        &DATE[..8],
        region,
        encoded,
    )
    .expect("signed");
    vec![
        ("policy".to_owned(), encoded.to_owned()),
        ("x-amz-algorithm".to_owned(), "AWS4-HMAC-SHA256".to_owned()),
        (
            "x-amz-credential".to_owned(),
            format!("{ACCESS_KEY}/{}/{region}/s3/aws4_request", &DATE[..8]),
        ),
        ("x-amz-date".to_owned(), DATE.to_owned()),
        ("x-amz-signature".to_owned(), signature),
    ]
}

/// The conditions every signed form here needs for its signing fields.
fn signing_conditions(state: &tessari_s3_api::ApiState) -> String {
    format!(
        r#"{{"x-amz-algorithm":"AWS4-HMAC-SHA256"}},{{"x-amz-credential":"{ACCESS_KEY}/{}/{}/s3/aws4_request"}},{{"x-amz-date":"{DATE}"}}"#,
        &DATE[..8],
        state.region()
    )
}

pub(crate) fn form(
    fields: &[(String, String)],
    filename: &str,
    file: &[u8],
    ending: &str,
) -> Vec<u8> {
    let mut body = Vec::new();
    for (name, value) in fields {
        body.extend_from_slice(
            format!(
                "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
            )
            .as_bytes(),
        );
    }
    body.extend_from_slice(
        format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(file);
    body.extend_from_slice(ending.as_bytes());
    body
}

pub(crate) fn closing() -> String {
    format!("\r\n--{BOUNDARY}--\r\n")
}

pub(crate) async fn post(state: &tessari_s3_api::ApiState, bucket: &str, body: Vec<u8>) -> Seen {
    let request = Request::builder()
        .method("POST")
        .uri(format!("/{bucket}"))
        .header("host", "localhost:9100")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .header("content-length", body.len().to_string())
        .body(Body::from(body))
        .expect("request");
    send(state, request).await
}

fn with(mut fields: Vec<(String, String)>, extra: &[(&str, &str)]) -> Vec<(String, String)> {
    for (name, value) in extra {
        fields.insert(0, ((*name).to_owned(), (*value).to_owned()));
    }
    fields
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_signed_form_uploads_its_file_under_the_expanded_key() {
    let _why = IGNORED;
    let (state, _pool) = fresh().await;
    assert_eq!(call(&state, "PUT", "/inbox", vec![], b"").await.status, 200);
    let encoded = policy(&format!(
        r#"{{"bucket":"inbox"}},["eq","$key","uploads/hello.txt"],["eq","$Content-Type","text/plain"],["starts-with","$x-amz-meta-from",""],["content-length-range",1,64],{{"success_action_status":"201"}},{}"#,
        signing_conditions(&state)
    ));
    let fields = with(
        signed(&state, &encoded),
        &[
            ("success_action_status", "201"),
            ("x-amz-meta-from", "browser"),
            ("Content-Type", "text/plain"),
            ("key", "uploads/${filename}"),
        ],
    );
    let seen = post(
        &state,
        "inbox",
        form(&fields, "hello.txt", b"hello form", &closing()),
    )
    .await;
    assert_eq!(seen.status, 201, "{}", seen.body);
    assert!(
        seen.body.contains("<Key>uploads/hello.txt</Key>"),
        "{}",
        seen.body
    );
    assert!(header(&seen, "etag").is_some());
    let read = call(&state, "GET", "/inbox/uploads/hello.txt", vec![], b"").await;
    assert_eq!((read.status, read.body.as_str()), (200, "hello form"));
    assert_eq!(header(&read, "content-type"), Some("text/plain"));
    assert_eq!(header(&read, "x-amz-meta-from"), Some("browser"));
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn every_way_out_of_the_policy_is_refused_and_nothing_is_stored() {
    let _why = IGNORED;
    let (state, _pool) = fresh().await;
    assert_eq!(call(&state, "PUT", "/inbox", vec![], b"").await.status, 200);
    let encoded = policy(&format!(
        r#"{{"bucket":"inbox"}},["starts-with","$key","uploads/"],["content-length-range",1,8],{}"#,
        signing_conditions(&state)
    ));
    let base = signed(&state, &encoded);
    let cases: Vec<Case> = vec![
        // The key is checked as expanded: here it is the file's name, outside the prefix.
        (
            "outside prefix",
            with(base.clone(), &[("key", "${filename}")]),
            "a.txt",
            b"x",
            closing(),
            "AccessDenied",
        ),
        (
            "uncovered field",
            with(base.clone(), &[("key", "uploads/a"), ("x-amz-meta-x", "1")]),
            "a",
            b"x",
            closing(),
            "AccessDenied",
        ),
        (
            "too large",
            with(base.clone(), &[("key", "uploads/a")]),
            "a",
            b"0123456789",
            closing(),
            "EntityTooLarge",
        ),
        (
            "bad ending",
            with(base.clone(), &[("key", "uploads/a")]),
            "a",
            b"x",
            format!("\r\n--{BOUNDARY}--XX"),
            "MalformedPOSTRequest",
        ),
    ];
    for (why, fields, filename, file, ending, code) in cases {
        let seen = post(&state, "inbox", form(&fields, filename, file, &ending)).await;
        assert_eq!(seen.code.as_deref(), Some(code), "{why}: {}", seen.body);
    }
    let mut forged = with(base.clone(), &[("key", "uploads/a")]);
    for (name, value) in &mut forged {
        if name == "x-amz-signature" {
            *value = "0".repeat(64);
        }
    }
    let seen = post(&state, "inbox", form(&forged, "a", b"x", &closing())).await;
    assert_eq!(
        seen.code.as_deref(),
        Some("SignatureDoesNotMatch"),
        "{}",
        seen.body
    );
    let listing = call(&state, "GET", "/inbox", vec![("list-type", Some("2"))], b"").await;
    assert!(!listing.body.contains("<Key>"), "{}", listing.body);
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn an_unsigned_form_is_anonymous_and_reaches_only_what_a_policy_grants_everyone() {
    let _why = IGNORED;
    let (state, _pool) = fresh().await;
    assert_eq!(call(&state, "PUT", "/drop", vec![], b"").await.status, 200);
    let fields = vec![("key".to_owned(), "box/a.txt".to_owned())];
    let refused = post(&state, "drop", form(&fields, "a.txt", b"x", &closing())).await;
    assert_eq!(
        refused.code.as_deref(),
        Some("AccessDenied"),
        "{}",
        refused.body
    );
    allow_public(&state, "drop").await;
    let open = r#"{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Principal":"*","Action":"s3:PutObject","Resource":"arn:aws:s3:::drop/box/*"}]}"#;
    assert_eq!(put_policy(&state, "drop", open).await.status, 204);
    let stored = post(&state, "drop", form(&fields, "a.txt", b"x", &closing())).await;
    assert_eq!(stored.status, 204, "{}", stored.body);
    let elsewhere = vec![("key".to_owned(), "other/a.txt".to_owned())];
    assert_eq!(
        post(&state, "drop", form(&elsewhere, "a.txt", b"x", &closing()))
            .await
            .code
            .as_deref(),
        Some("AccessDenied")
    );
    let read = call(&state, "GET", "/drop/box/a.txt", vec![], b"").await;
    assert_eq!((read.status, read.body.as_str()), (200, "x"));
}
