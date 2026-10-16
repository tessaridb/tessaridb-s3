use axum::http::{HeaderMap, HeaderName, HeaderValue};
use tessari_s3_core::objects::checksum::ChecksumAlgorithm;
use tessari_s3_storage::objects::WriteCondition;
use tessari_s3_types::ErrorCode;

use super::read_put_headers;

fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
    pairs
        .iter()
        .map(|(n, v)| {
            (
                HeaderName::from_bytes(n.as_bytes()).expect("name"),
                HeaderValue::from_str(v).expect("value"),
            )
        })
        .collect()
}

fn refused(pairs: &[(&str, &str)]) -> ErrorCode {
    read_put_headers(&headers(pairs))
        .map(|_| ())
        .expect_err("refused")
        .code
}

#[test]
fn system_headers_and_metadata_are_kept_and_aws_chunked_is_removed() {
    let read = read_put_headers(&headers(&[
        ("content-type", "text/plain"),
        ("content-encoding", "aws-chunked, gzip"),
        ("x-amz-meta-Color", "blue"),
        ("cache-control", "no-cache"),
        ("x-amz-date", "20130524T000000Z"),
    ]))
    .expect("reads");
    assert_eq!(
        read.system.get("content-type").map(String::as_str),
        Some("text/plain")
    );
    assert_eq!(
        read.system.get("content-encoding").map(String::as_str),
        Some("gzip")
    );
    assert_eq!(
        read.metadata.get("color").map(String::as_str),
        Some("blue"),
        "names are lowercase"
    );
    assert_eq!(read.condition, WriteCondition::None);
    let only_chunked =
        read_put_headers(&headers(&[("content-encoding", "aws-chunked")])).expect("reads");
    assert!(!only_chunked.system.contains_key("content-encoding"));
}

#[test]
fn semantics_this_server_does_not_implement_are_refused_not_dropped() {
    assert_eq!(
        refused(&[("x-amz-server-side-encryption", "AES256")]),
        ErrorCode::NotImplemented
    );
    assert_eq!(
        refused(&[("x-amz-object-lock-event-hold", "ON")]),
        ErrorCode::NotImplemented
    );
    assert_eq!(
        refused(&[("x-amz-website-redirect-location", "/x")]),
        ErrorCode::NotImplemented
    );
    assert_eq!(
        refused(&[("x-amz-checksum-sha512", "AAAA")]),
        ErrorCode::NotImplemented
    );
    assert_eq!(
        refused(&[("x-amz-acl", "public-read")]),
        ErrorCode::AccessControlListNotSupported
    );
    assert_eq!(
        refused(&[("x-amz-grant-read", "id=x")]),
        ErrorCode::AccessControlListNotSupported
    );
    assert_eq!(
        refused(&[("x-amz-storage-class", "GLACIER")]),
        ErrorCode::InvalidStorageClass
    );
    assert_eq!(
        refused(&[("if-none-match", "\"abc\"")]),
        ErrorCode::NotImplemented
    );
    assert!(
        read_put_headers(&headers(&[
            ("x-amz-acl", "private"),
            ("x-amz-storage-class", "STANDARD")
        ]))
        .is_ok()
    );
}

#[test]
fn user_metadata_is_bounded_at_two_kilobytes() {
    let value = "v".repeat(2_040);
    assert!(
        read_put_headers(&headers(&[("x-amz-meta-k", &value)])).is_ok(),
        "2041 bytes is within"
    );
    let value = "v".repeat(2_048);
    assert_eq!(
        refused(&[("x-amz-meta-k", &value)]),
        ErrorCode::MetadataTooLarge
    );
}

#[test]
fn one_checksum_is_declared_and_conflicting_ones_are_refused() {
    let read = read_put_headers(&headers(&[
        ("x-amz-checksum-crc32", "NhCmhg=="),
        ("x-amz-sdk-checksum-algorithm", "CRC32"),
    ]))
    .expect("reads");
    let declared = read.integrity.checksum.expect("declared");
    assert_eq!(
        (declared.algorithm, declared.header_value.as_deref()),
        (ChecksumAlgorithm::Crc32, Some("NhCmhg=="))
    );
    let trailer = read_put_headers(&headers(&[("x-amz-trailer", "x-amz-checksum-crc64nvme")]))
        .expect("reads");
    assert_eq!(
        trailer
            .integrity
            .checksum
            .map(|d| (d.algorithm, d.header_value)),
        Some((ChecksumAlgorithm::Crc64Nvme, None))
    );
    assert_eq!(
        refused(&[
            ("x-amz-checksum-crc32", "a"),
            ("x-amz-sdk-checksum-algorithm", "SHA256")
        ]),
        ErrorCode::InvalidRequest
    );
    assert_eq!(
        refused(&[("x-amz-checksum-crc32", "a"), ("x-amz-checksum-sha1", "b")]),
        ErrorCode::InvalidRequest
    );
}

#[test]
fn conditions_and_content_md5_parse() {
    assert_eq!(
        read_put_headers(&headers(&[("if-none-match", "*")])).map(|r| r.condition),
        Ok(WriteCondition::IfNoneMatch)
    );
    assert_eq!(
        read_put_headers(&headers(&[("if-match", "abc")])).map(|r| r.condition),
        Ok(WriteCondition::IfMatch("\"abc\"".to_owned())),
        "a bare tag is quoted"
    );
    assert_eq!(
        refused(&[("content-md5", "not-base64!")]),
        ErrorCode::InvalidDigest
    );
    let md5 = read_put_headers(&headers(&[("content-md5", "XUFAKrxLKna5cZ2REBfFkg==")]))
        .expect("reads")
        .integrity
        .content_md5;
    assert_eq!(md5.map(|m| m[0]), Some(0x5d));
}

#[test]
fn an_upload_declaring_its_algorithm_and_type_declares_no_checksum_value() {
    let read = read_put_headers(&headers(&[
        ("x-amz-checksum-algorithm", "SHA256"),
        ("x-amz-checksum-type", "COMPOSITE"),
    ]))
    .expect("an upload's declaration is not a checksum header");
    assert_eq!(read.integrity.checksum, None);
}

#[test]
fn object_lock_headers_ask_for_a_retention_and_a_hold_or_are_refused_as_invalid() {
    use tessari_s3_core::object_lock::Mode;

    let asked = read_put_headers(&headers(&[
        ("x-amz-object-lock-mode", "COMPLIANCE"),
        (
            "x-amz-object-lock-retain-until-date",
            "2999-01-01T00:00:00Z",
        ),
        ("x-amz-object-lock-legal-hold", "ON"),
    ]))
    .expect("reads")
    .lock;
    assert_eq!(
        asked.retention.map(|(mode, _)| mode),
        Some(Mode::Compliance)
    );
    assert!(asked.legal_hold && asked.asked);
    let off = read_put_headers(&headers(&[("x-amz-object-lock-legal-hold", "OFF")]))
        .expect("reads")
        .lock;
    assert!(!off.legal_hold && off.asked && off.retention.is_none());
    assert!(!read_put_headers(&headers(&[])).expect("reads").lock.asked);
    for pairs in [
        &[("x-amz-object-lock-mode", "GOVERNANCE")][..],
        &[(
            "x-amz-object-lock-retain-until-date",
            "2999-01-01T00:00:00Z",
        )],
        &[
            ("x-amz-object-lock-mode", "FOREVER"),
            (
                "x-amz-object-lock-retain-until-date",
                "2999-01-01T00:00:00Z",
            ),
        ],
        &[
            ("x-amz-object-lock-mode", "GOVERNANCE"),
            (
                "x-amz-object-lock-retain-until-date",
                "2001-01-01T00:00:00Z",
            ),
        ],
        &[("x-amz-object-lock-legal-hold", "MAYBE")],
    ] {
        assert_eq!(refused(pairs), ErrorCode::InvalidArgument, "{pairs:?}");
    }
}

#[test]
fn the_tagging_header_is_read_as_tags_or_refused_as_an_invalid_tag() {
    let read = read_put_headers(&headers(&[("x-amz-tagging", "team=blue&tier=gold%20plus")]))
        .expect("reads");
    assert_eq!(
        read.tags
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect::<Vec<_>>(),
        [("team", "blue"), ("tier", "gold plus")]
    );
    assert!(
        read_put_headers(&headers(&[]))
            .expect("reads")
            .tags
            .is_empty()
    );
    let eleven: Vec<String> = (0..11).map(|n| format!("k{n}=v")).collect();
    for header in [eleven.join("&").as_str(), "aws:x=1", "a=%Z1", "a=1&a=2"] {
        assert_eq!(
            refused(&[("x-amz-tagging", header)]),
            ErrorCode::InvalidTag,
            "{header}"
        );
    }
}
