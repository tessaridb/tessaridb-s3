use axum::http::{HeaderMap, HeaderName, HeaderValue};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use md5::{Digest, Md5};
use tessari_s3_types::ErrorCode;

use super::{ALGORITHM, KEY, KEY_MD5, answer_customer, read_customer};

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

fn encoded(key: &[u8]) -> (String, String) {
    (STANDARD.encode(key), STANDARD.encode(Md5::digest(key)))
}

#[test]
fn a_request_bringing_no_customer_key_reads_as_none() {
    assert!(
        read_customer(&headers(&[("x-amz-server-side-encryption", "AES256")]))
            .expect("reads")
            .is_none()
    );
}

#[test]
fn a_well_formed_customer_key_is_read_with_its_md5() {
    let (key, md5) = encoded(&[7_u8; 32]);
    let customer = read_customer(&headers(&[
        (ALGORITHM, "AES256"),
        (KEY, &key),
        (KEY_MD5, &md5),
    ]))
    .expect("reads")
    .expect("a key");
    assert_eq!(customer.md5, md5);
    let mut answered = HeaderMap::new();
    answer_customer(&mut answered, &customer);
    assert_eq!(
        answered.get(ALGORITHM).map(|v| v.as_bytes()),
        Some(&b"AES256"[..])
    );
    assert_eq!(
        answered.get(KEY_MD5).map(|v| v.as_bytes()),
        Some(md5.as_bytes())
    );
    assert_eq!(answered.get(KEY), None, "the key is never answered");
}

#[test]
fn a_customer_key_that_is_incomplete_or_malformed_is_refused_without_naming_it() {
    let (key, md5) = encoded(&[7_u8; 32]);
    let (short, short_md5) = encoded(&[7_u8; 16]);
    let (_, other_md5) = encoded(&[8_u8; 32]);
    for (pairs, code) in [
        (
            vec![(KEY, key.as_str()), (KEY_MD5, md5.as_str())],
            ErrorCode::InvalidArgument,
        ),
        (
            vec![(ALGORITHM, "AES256"), (KEY_MD5, md5.as_str())],
            ErrorCode::InvalidArgument,
        ),
        (
            vec![(ALGORITHM, "AES256"), (KEY, key.as_str())],
            ErrorCode::InvalidArgument,
        ),
        (
            vec![
                (ALGORITHM, "AES128"),
                (KEY, key.as_str()),
                (KEY_MD5, md5.as_str()),
            ],
            ErrorCode::InvalidEncryptionAlgorithmError,
        ),
        (
            vec![
                (ALGORITHM, "AES256"),
                (KEY, short.as_str()),
                (KEY_MD5, short_md5.as_str()),
            ],
            ErrorCode::InvalidArgument,
        ),
        (
            vec![
                (ALGORITHM, "AES256"),
                (KEY, "not base64!"),
                (KEY_MD5, md5.as_str()),
            ],
            ErrorCode::InvalidArgument,
        ),
        (
            vec![
                (ALGORITHM, "AES256"),
                (KEY, key.as_str()),
                (KEY_MD5, other_md5.as_str()),
            ],
            ErrorCode::InvalidArgument,
        ),
    ] {
        let error = read_customer(&headers(&pairs))
            .map(|_| ())
            .expect_err("refused");
        assert_eq!(error.code, code, "{pairs:?}");
        assert!(!error.message.contains(&key), "{}", error.message);
    }
}
