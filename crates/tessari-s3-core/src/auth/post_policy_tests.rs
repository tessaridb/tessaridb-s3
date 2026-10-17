//! A browser form's policy (S3 ref 03 M7): parsed from its base64 JSON, checked against the EXPANDED form with every
//! field covered, and its signature verified with the SigV4 signing key.

use base64::Engine as _;

use super::SecretKey;
use super::post_policy::{PostPolicy, PostPolicyError, post_signature, verify_post};

/// A bucket, a form and the refusal it earns.
type Case = (
    &'static str,
    Vec<(&'static str, &'static str)>,
    PostPolicyError,
);

const NOW: i64 = 1_792_000_000; // 2026-10-13T...Z

fn encode(json: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(json)
}

fn policy(conditions: &str) -> PostPolicy {
    PostPolicy::parse(&encode(&format!(
        r#"{{"expiration":"2030-01-01T00:00:00.000Z","conditions":[{conditions}]}}"#
    )))
    .expect("parsed")
}

fn fields(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
        .collect()
}

#[test]
fn a_form_every_condition_holds_for_passes_and_names_its_size_range() {
    let policy = policy(
        r#"{"bucket":"photos"},["starts-with","$key","uploads/"],{"x-amz-algorithm":"AWS4-HMAC-SHA256"},
        ["eq","$Content-Type","image/png"],["content-length-range",1,1048576],["starts-with","$x-amz-meta-tag",""]"#,
    );
    let form = fields(&[
        ("key", "uploads/cat.png"),
        ("x-amz-algorithm", "AWS4-HMAC-SHA256"),
        ("content-type", "image/png"),
        ("x-amz-meta-tag", "anything"),
        ("x-ignore-note", "skipped"),
        ("policy", "…"),
        ("x-amz-signature", "…"),
    ]);
    assert_eq!(policy.check("photos", &form, NOW), Ok(Some((1, 1_048_576))));
}

#[test]
fn each_way_a_form_can_leave_its_policy_is_refused_by_name() {
    let policy =
        policy(r#"{"bucket":"photos"},["starts-with","$key","uploads/"],{"acl":"private"}"#);
    let good = [("key", "uploads/a"), ("acl", "private")];
    assert_eq!(policy.check("photos", &fields(&good), NOW), Ok(None));
    let cases: [Case; 4] = [
        (
            "photos",
            vec![("key", "other/a"), ("acl", "private")],
            PostPolicyError::ConditionFailed("key".to_owned()),
        ),
        (
            "photos",
            vec![("key", "uploads/a"), ("acl", "public-read")],
            PostPolicyError::ConditionFailed("acl".to_owned()),
        ),
        (
            "photos",
            vec![
                ("key", "uploads/a"),
                ("acl", "private"),
                ("success_action_status", "201"),
            ],
            PostPolicyError::NotCovered("success_action_status".to_owned()),
        ),
        (
            "elsewhere",
            good.to_vec(),
            PostPolicyError::ConditionFailed("bucket".to_owned()),
        ),
    ];
    for (bucket, form, error) in cases {
        assert_eq!(policy.check(bucket, &fields(&form), NOW), Err(error));
    }
    let expired = PostPolicy::parse(&encode(
        r#"{"expiration":"2020-01-01T00:00:00Z","conditions":[["starts-with","$key",""]]}"#,
    ))
    .expect("parsed");
    assert_eq!(
        expired.check("photos", &fields(&[("key", "a")]), NOW),
        Err(PostPolicyError::Expired)
    );
}

#[test]
fn a_policy_this_server_cannot_hold_to_its_word_is_refused_when_read() {
    for bad in [
        "not base64!",
        &encode("[]"),
        &encode(r#"{"conditions":[]}"#),
        &encode(r#"{"expiration":"tomorrow","conditions":[]}"#),
        &encode(r#"{"expiration":"2030-01-01T00:00:00Z","conditions":[["matches","$key","a"]]}"#),
        &encode(
            r#"{"expiration":"2030-01-01T00:00:00Z","conditions":[["content-length-range",10,1]]}"#,
        ),
        &encode(r#"{"expiration":"2030-01-01T00:00:00Z","conditions":[{"a":"b","c":"d"}]}"#),
    ] {
        assert!(
            matches!(PostPolicy::parse(bad), Err(PostPolicyError::Malformed(_))),
            "{bad}"
        );
    }
}

#[test]
fn the_signature_is_the_hmac_of_the_encoded_policy_under_the_signing_key() {
    let secret = SecretKey::new("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_owned());
    let encoded = encode(r#"{"expiration":"2030-01-01T00:00:00Z","conditions":[]}"#);
    let credential = "AKIAIOSFODNN7EXAMPLE/20261013/us-east-1/s3/aws4_request";
    let date = "20261013T000000Z";
    let signature = post_signature(&secret, "20261013", "us-east-1", &encoded).expect("signed");
    let verified = verify_post(
        &encoded,
        "AWS4-HMAC-SHA256",
        credential,
        date,
        &signature,
        &secret,
        "us-east-1",
    )
    .expect("verified");
    assert_eq!(verified.access_key(), "AKIAIOSFODNN7EXAMPLE");
    let mut tampered = encoded.clone();
    tampered.push('A');
    assert!(
        verify_post(
            &tampered,
            "AWS4-HMAC-SHA256",
            credential,
            date,
            &signature,
            &secret,
            "us-east-1"
        )
        .is_err()
    );
    assert!(
        verify_post(
            &encoded,
            "AWS4-HMAC-SHA256",
            credential,
            date,
            &signature,
            &secret,
            "eu-west-1"
        )
        .is_err(),
        "the scope names another region"
    );
}
