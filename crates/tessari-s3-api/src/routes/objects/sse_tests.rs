use axum::http::{HeaderMap, HeaderName, HeaderValue};
use tessari_s3_storage::encryption::{KeyName, Sealing};
use tessari_s3_types::ErrorCode;

use super::{answer_sse, etag_for, read_sse};

fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
    pairs
        .iter()
        .map(|(name, value)| {
            (
                HeaderName::from_bytes(name.as_bytes()).expect("name"),
                HeaderValue::from_str(value).expect("value"),
            )
        })
        .collect()
}

fn named(name: &str) -> Option<Sealing> {
    Some(Sealing::Named(KeyName::new(name).expect("usable")))
}

fn refused(pairs: &[(&str, &str)]) -> ErrorCode {
    read_sse(&headers(pairs)).expect_err("refused").code
}

const MODE: &str = "x-amz-server-side-encryption";
const KEY_ID: &str = "x-amz-server-side-encryption-aws-kms-key-id";

#[test]
fn a_request_asks_for_no_key_the_servers_or_a_named_one() {
    assert_eq!(read_sse(&headers(&[])).expect("reads"), None);
    assert_eq!(
        read_sse(&headers(&[(MODE, "AES256")])).expect("reads"),
        Some(Sealing::Server)
    );
    assert_eq!(
        read_sse(&headers(&[(MODE, "aws:kms")])).expect("reads"),
        named("aws-s3"),
        "no key id names S3's own managed key"
    );
    for id in [
        "tenant-7",
        "key/tenant-7",
        "alias/tenant-7",
        "arn:aws:kms:eu-west-1:000000000000:key/tenant-7",
        "arn:aws:kms:eu-west-1:000000000000:alias/tenant-7",
    ] {
        assert_eq!(
            read_sse(&headers(&[(MODE, "aws:kms"), (KEY_ID, id)])).expect("reads"),
            named("tenant-7"),
            "{id}"
        );
    }
    assert_eq!(
        read_sse(&headers(&[(MODE, "aws:kms"), (KEY_ID, "alias/aws/s3")])).expect("reads"),
        named("aws-s3")
    );
    assert_eq!(
        read_sse(&headers(&[
            (MODE, "aws:kms"),
            ("x-amz-server-side-encryption-bucket-key-enabled", "true")
        ]))
        .expect("reads"),
        named("aws-s3"),
        "a bucket key changes cost, not protection"
    );
}

#[test]
fn what_this_server_cannot_do_or_s3_does_not_define_is_refused_by_name() {
    for (pairs, code) in [
        (vec![(MODE, "rot13")], ErrorCode::InvalidArgument),
        (vec![(KEY_ID, "tenant-7")], ErrorCode::InvalidArgument),
        (
            vec![(MODE, "AES256"), (KEY_ID, "tenant-7")],
            ErrorCode::InvalidArgument,
        ),
        (
            vec![(MODE, "aws:kms"), (KEY_ID, "Bad_Name")],
            ErrorCode::InvalidArgument,
        ),
        (
            vec![(MODE, "aws:kms"), (KEY_ID, "alias/a;b")],
            ErrorCode::InvalidArgument,
        ),
        (vec![(MODE, "aws:kms:dsse")], ErrorCode::NotImplemented),
        (
            vec![
                (MODE, "aws:kms"),
                ("x-amz-server-side-encryption-context", "e30="),
            ],
            ErrorCode::NotImplemented,
        ),
    ] {
        assert_eq!(refused(&pairs), code, "{pairs:?}");
    }
}

#[test]
fn a_response_says_the_mode_and_for_a_named_key_its_name() {
    let mut answered = HeaderMap::new();
    answer_sse(&mut answered, &Sealing::Server);
    assert_eq!(
        answered.get(MODE).map(|v| v.as_bytes()),
        Some(&b"AES256"[..])
    );
    assert_eq!(answered.get(KEY_ID), None);
    let mut answered = HeaderMap::new();
    answer_sse(&mut answered, &named("tenant-7").expect("named"));
    assert_eq!(
        answered.get(MODE).map(|v| v.as_bytes()),
        Some(&b"aws:kms"[..])
    );
    assert_eq!(
        answered.get(KEY_ID).map(|v| v.as_bytes()),
        Some(&b"tenant-7"[..])
    );
}

#[test]
fn under_a_named_key_the_etag_is_no_md5_of_the_plaintext() {
    let md5 = "5d41402abc4b2a76b9719d911017c592";
    assert_eq!(etag_for(md5, None).expect("etag"), format!("\"{md5}\""));
    assert_eq!(
        etag_for(md5, Some(&Sealing::Server)).expect("etag"),
        format!("\"{md5}\""),
        "SSE-S3 keeps the MD5"
    );
    let kms = named("tenant-7");
    let first = etag_for(md5, kms.as_ref()).expect("etag");
    let second = etag_for(md5, kms.as_ref()).expect("etag");
    assert!(!first.contains(md5), "{first}");
    assert_ne!(
        first, second,
        "two writes of the same bytes do not share an ETag"
    );
    assert_eq!(first.len(), 34, "a quoted 128-bit value: {first}");
}
