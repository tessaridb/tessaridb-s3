//! Header authentication: GET with Range, PUT with a body, a valueless subresource, a sorted query.

use tessari_s3_core::auth::{AuthError, PayloadHash, PayloadVerifier, verify_payload_digest};
use tessari_s3_core::objects::checksum::Hashes;

use crate::{EMPTY_SHA256, NOW, REGION, authorization, check};

// ---- GET object with Range (header authentication) ----

const GET_SIGNATURE: &str = "f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41";

fn get_object_headers(authorization: &str, range: &'static str) -> Vec<(&'static str, String)> {
    vec![
        ("Host", "examplebucket.s3.amazonaws.com".to_owned()),
        ("Authorization", authorization.to_owned()),
        ("Range", range.to_owned()),
        ("x-amz-content-sha256", EMPTY_SHA256.to_owned()),
        ("x-amz-date", "20130524T000000Z".to_owned()),
    ]
}

#[test]
fn get_object_with_range_reproduces_the_published_signature() {
    let owned = get_object_headers(
        &authorization("host;range;x-amz-content-sha256;x-amz-date", GET_SIGNATURE),
        "bytes=0-9",
    );
    let verified =
        check("GET", "/test.txt", "", &owned, REGION, NOW).expect("published example verifies");
    assert_eq!(verified.access_key(), "AKIAIOSFODNN7EXAMPLE");
    let expected: [u8; 32] =
        std::array::from_fn(|i| u8::from_str_radix(&EMPTY_SHA256[i * 2..i * 2 + 2], 16).unwrap());
    assert_eq!(verified.payload(), PayloadHash::Sha256(expected));
}

#[test]
fn get_object_with_one_header_value_changed_is_refused() {
    let owned = get_object_headers(
        &authorization("host;range;x-amz-content-sha256;x-amz-date", GET_SIGNATURE),
        "bytes=0-10",
    );
    assert_eq!(
        check("GET", "/test.txt", "", &owned, REGION, NOW).map(|_| ()),
        Err(AuthError::SignatureMismatch)
    );
}

#[test]
fn get_object_signed_for_another_region_is_refused() {
    let owned = get_object_headers(
        &authorization("host;range;x-amz-content-sha256;x-amz-date", GET_SIGNATURE),
        "bytes=0-9",
    );
    let refusal = check("GET", "/test.txt", "", &owned, "us-west-2", NOW).map(|_| ());
    assert!(
        matches!(refusal, Err(AuthError::ScopeMismatch { .. })),
        "{refusal:?}"
    );
}

#[test]
fn get_object_outside_the_fifteen_minute_window_is_refused() {
    let owned = get_object_headers(
        &authorization("host;range;x-amz-content-sha256;x-amz-date", GET_SIGNATURE),
        "bytes=0-9",
    );
    assert!(
        check("GET", "/test.txt", "", &owned, REGION, NOW + 900).is_ok(),
        "the window is inclusive at 15 minutes"
    );
    for now in [NOW + 901, NOW - 901] {
        assert_eq!(
            check("GET", "/test.txt", "", &owned, REGION, now).map(|_| ()),
            Err(AuthError::ClockSkew),
            "{now}"
        );
    }
}

#[test]
fn an_x_amz_header_left_out_of_signed_headers_is_refused() {
    let mut owned = get_object_headers(
        &authorization("host;range;x-amz-content-sha256;x-amz-date", GET_SIGNATURE),
        "bytes=0-9",
    );
    owned.push(("x-amz-meta-added", "1".to_owned()));
    let refusal = check("GET", "/test.txt", "", &owned, REGION, NOW).map(|_| ());
    assert_eq!(
        refusal,
        Err(AuthError::HeaderNotSigned {
            name: "x-amz-meta-added".to_owned()
        })
    );
}

#[test]
fn a_signature_differing_in_one_digit_is_refused() {
    let mut forged = GET_SIGNATURE.to_owned();
    forged.replace_range(63..64, "0");
    let owned = get_object_headers(
        &authorization("host;range;x-amz-content-sha256;x-amz-date", &forged),
        "bytes=0-9",
    );
    assert_eq!(
        check("GET", "/test.txt", "", &owned, REGION, NOW).map(|_| ()),
        Err(AuthError::SignatureMismatch)
    );
}

// ---- PUT object: a `$` in the key, a Date header, a body hash ----

const PUT_BODY: &[u8] = b"Welcome to Amazon S3.";
const PUT_SHA256: &str = "44ce7dd67c959e0d3524ffac1771dfbba87d2b6b4b4e99e42034a8b803f8b072";

fn put_object_headers() -> Vec<(&'static str, String)> {
    let signature = "98ad721746da40c64f1a55b78f14c238d841ea1380cd77a1b5971af0ece108bd";
    vec![
        ("Host", "examplebucket.s3.amazonaws.com".to_owned()),
        ("Date", "Fri, 24 May 2013 00:00:00 GMT".to_owned()),
        (
            "Authorization",
            authorization(
                "date;host;x-amz-content-sha256;x-amz-date;x-amz-storage-class",
                signature,
            ),
        ),
        ("x-amz-date", "20130524T000000Z".to_owned()),
        ("x-amz-storage-class", "REDUCED_REDUNDANCY".to_owned()),
        ("x-amz-content-sha256", PUT_SHA256.to_owned()),
    ]
}

#[test]
fn put_object_reproduces_the_published_signature_and_its_body_verifies() {
    let verified = check(
        "PUT",
        "/test%24file.text",
        "",
        &put_object_headers(),
        REGION,
        NOW,
    )
    .expect("published example verifies");
    let PayloadHash::Sha256(expected) = verified.payload() else {
        panic!("a hex payload hash")
    };
    let mut body = PayloadVerifier::new(expected);
    body.update(&PUT_BODY[..7]);
    body.update(&PUT_BODY[7..]);
    assert_eq!(body.finish(), Ok(()));
}

#[test]
fn put_object_with_one_body_byte_changed_is_refused_before_commit() {
    let verified = check(
        "PUT",
        "/test%24file.text",
        "",
        &put_object_headers(),
        REGION,
        NOW,
    )
    .expect("headers verify");
    let PayloadHash::Sha256(expected) = verified.payload() else {
        panic!("a hex payload hash")
    };
    let mut body = PayloadVerifier::new(expected);
    body.update(b"Welcome to Amazon S4.");
    assert_eq!(body.finish(), Err(AuthError::PayloadHashMismatch));
}

#[test]
fn put_object_body_is_verified_from_the_one_pass_digests() {
    let verified = check(
        "PUT",
        "/test%24file.text",
        "",
        &put_object_headers(),
        REGION,
        NOW,
    )
    .expect("headers verify");
    let PayloadHash::Sha256(expected) = verified.payload() else {
        panic!("a hex payload hash")
    };
    let mut same = Hashes::new();
    same.update(PUT_BODY);
    assert_eq!(
        verify_payload_digest(&expected, &same.finish().sha256),
        Ok(())
    );
    let mut other = Hashes::new();
    other.update(b"Welcome to Amazon S4.");
    assert_eq!(
        verify_payload_digest(&expected, &other.finish().sha256),
        Err(AuthError::PayloadHashMismatch)
    );
}

#[test]
fn put_object_to_a_different_key_is_refused() {
    let refusal = check(
        "PUT",
        "/test%24file.txt",
        "",
        &put_object_headers(),
        REGION,
        NOW,
    )
    .map(|_| ());
    assert_eq!(refusal, Err(AuthError::SignatureMismatch));
}

// ---- bucket subresource and list query ----

fn bucket_get_headers(signature: &str) -> Vec<(&'static str, String)> {
    vec![
        ("Host", "examplebucket.s3.amazonaws.com".to_owned()),
        (
            "Authorization",
            authorization("host;x-amz-content-sha256;x-amz-date", signature),
        ),
        ("x-amz-date", "20130524T000000Z".to_owned()),
        ("x-amz-content-sha256", EMPTY_SHA256.to_owned()),
    ]
}

#[test]
fn a_valueless_subresource_signs_as_name_equals() {
    let owned =
        bucket_get_headers("fea454ca298b7da1c68078a5d1bdbfbbe0d65c699e0f91ac7a200a0136783543");
    assert!(check("GET", "/", "lifecycle", &owned, REGION, NOW).is_ok());
}

#[test]
fn query_parameters_are_sorted_after_encoding() {
    let owned =
        bucket_get_headers("34b48302e7b5fa45bde8084f4b7868a86f0a534bc59db6670ed5711ef69dc6f7");
    assert!(
        check("GET", "/", "prefix=J&max-keys=2", &owned, REGION, NOW).is_ok(),
        "arrival order does not matter"
    );
    assert_eq!(
        check("GET", "/", "max-keys=3&prefix=J", &owned, REGION, NOW).map(|_| ()),
        Err(AuthError::SignatureMismatch)
    );
}
