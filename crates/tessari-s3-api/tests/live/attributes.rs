//! GetObjectAttributes against a real node: the attributes `x-amz-object-attributes` names and no others — the ETag
//! unquoted, the stored checksum and its type, a multipart object's parts paged by `x-amz-max-parts` and
//! `x-amz-part-number-marker`, the storage class and the size — of the current version or the one named; asked as
//! both `s3:GetObjectAttributes` and `s3:GetObject`.

use tessari_s3_core::authz::Role;

use crate::access::user_with_key;
use crate::complete::{complete, document};
use crate::large::body;
use crate::multipart::{create, etag, md5_hex, part};
use crate::policy::put_policy;
use crate::versions_kept::{put, set_versioning, version_header};
use crate::{IGNORED, Seen, call, call_as, call_with, fresh};

const MIB: usize = 1 << 20;
const ALL: &str = "ETag,Checksum,ObjectParts,StorageClass,ObjectSize";

fn header<'a>(seen: &'a Seen, name: &str) -> Option<&'a str> {
    seen.headers.get(name).and_then(|value| value.to_str().ok())
}

async fn attributes(
    state: &tessari_s3_api::ApiState,
    path: &str,
    query: Vec<(&str, Option<&str>)>,
    headers: Vec<(&str, &str)>,
) -> Seen {
    let mut query = query;
    query.push(("attributes", None));
    call_with(state, "GET", path, query, headers, b"").await
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_single_part_object_answers_the_attributes_asked_for() {
    let _why = IGNORED;
    let (state, _pool) = fresh().await;
    assert_eq!(call(&state, "PUT", "/attrs", vec![], b"").await.status, 200);
    let written = call(&state, "PUT", "/attrs/k", vec![], b"foo").await;
    assert_eq!(etag(&written), format!("\"{}\"", md5_hex(b"foo")));
    let seen = attributes(
        &state,
        "/attrs/k",
        vec![],
        vec![("x-amz-object-attributes", ALL)],
    )
    .await;
    assert_eq!(seen.status, 200, "{}", seen.body);
    assert!(
        seen.body.contains("<GetObjectAttributesResponse"),
        "{}",
        seen.body
    );
    assert!(
        seen.body
            .contains(&format!("<ETag>{}</ETag>", md5_hex(b"foo"))),
        "{}",
        seen.body
    );
    assert!(seen.body.contains("<ChecksumCRC64NVME>"), "{}", seen.body);
    assert!(
        seen.body
            .contains("<ChecksumType>FULL_OBJECT</ChecksumType>"),
        "{}",
        seen.body
    );
    assert!(
        seen.body.contains("<StorageClass>STANDARD</StorageClass>"),
        "{}",
        seen.body
    );
    assert!(
        seen.body.contains("<ObjectSize>3</ObjectSize>"),
        "{}",
        seen.body
    );
    assert!(!seen.body.contains("<ObjectParts>"), "{}", seen.body);
    assert!(header(&seen, "last-modified").is_some());
    assert!(header(&seen, "x-amz-version-id").is_none());

    let size = attributes(
        &state,
        "/attrs/k",
        vec![],
        vec![("x-amz-object-attributes", "ObjectSize")],
    )
    .await;
    assert!(
        size.body.contains("<ObjectSize>3</ObjectSize>"),
        "{}",
        size.body
    );
    assert!(
        !size.body.contains("<ETag>"),
        "only what was asked for: {}",
        size.body
    );
    let unknown = attributes(
        &state,
        "/attrs/k",
        vec![],
        vec![("x-amz-object-attributes", "Colour")],
    )
    .await;
    assert_eq!(
        unknown.code.as_deref(),
        Some("InvalidArgument"),
        "{}",
        unknown.body
    );
    let missing = attributes(
        &state,
        "/attrs/none",
        vec![],
        vec![("x-amz-object-attributes", ALL)],
    )
    .await;
    assert_eq!(
        missing.code.as_deref(),
        Some("NoSuchKey"),
        "{}",
        missing.body
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_multipart_object_lists_its_parts_page_by_page() {
    let _why = IGNORED;
    let (state, _pool) = fresh().await;
    assert_eq!(
        call(&state, "PUT", "/attrmp", vec![], b"").await.status,
        200
    );
    let upload = create(&state, "/attrmp/k").await;
    let mut listed = Vec::new();
    for (number, bytes) in [
        ("1", body(5 * MIB, 1)),
        ("2", body(5 * MIB, 2)),
        ("3", b"tail".to_vec()),
    ] {
        let uploaded = part(&state, "/attrmp/k", &upload, number, &bytes).await;
        assert_eq!(uploaded.status, 200, "{}", uploaded.body);
        listed.push((number, etag(&uploaded)));
    }
    let parts: Vec<(&str, &str)> = listed.iter().map(|(n, e)| (*n, e.as_str())).collect();
    let done = complete(&state, "/attrmp/k", &upload, vec![], &document(&parts)).await;
    assert_eq!(done.status, 200, "{}", done.body);
    let all = attributes(
        &state,
        "/attrmp/k",
        vec![],
        vec![("x-amz-object-attributes", ALL)],
    )
    .await;
    assert_eq!(all.status, 200, "{}", all.body);
    assert!(
        all.body.contains("<PartsCount>3</PartsCount>"),
        "{}",
        all.body
    );
    assert!(
        all.body.contains("<IsTruncated>false</IsTruncated>"),
        "{}",
        all.body
    );
    assert_eq!(all.body.matches("<Part>").count(), 3, "{}", all.body);
    let size = 2 * 5 * MIB + 4;
    assert!(
        all.body
            .contains(&format!("<ObjectSize>{size}</ObjectSize>")),
        "{}",
        all.body
    );
    assert!(
        all.body.contains("-3</ETag>"),
        "the multipart ETag: {}",
        all.body
    );
    let page = attributes(
        &state,
        "/attrmp/k",
        vec![],
        vec![
            ("x-amz-object-attributes", "ObjectParts"),
            ("x-amz-max-parts", "1"),
            ("x-amz-part-number-marker", "1"),
        ],
    )
    .await;
    assert_eq!(page.status, 200, "{}", page.body);
    assert_eq!(page.body.matches("<Part>").count(), 1, "{}", page.body);
    assert!(
        page.body.contains("<PartNumber>2</PartNumber>"),
        "{}",
        page.body
    );
    assert!(
        page.body.contains(&format!("<Size>{}</Size>", 5 * MIB)),
        "{}",
        page.body
    );
    assert!(
        page.body.contains("<IsTruncated>true</IsTruncated>"),
        "{}",
        page.body
    );
    assert!(
        page.body
            .contains("<NextPartNumberMarker>2</NextPartNumberMarker>"),
        "{}",
        page.body
    );
    assert!(
        page.body.contains("<PartNumberMarker>1</PartNumberMarker>"),
        "{}",
        page.body
    );
    assert!(
        page.body.contains("<MaxParts>1</MaxParts>"),
        "{}",
        page.body
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_named_version_answers_its_own_attributes() {
    let _why = IGNORED;
    let (state, _pool) = fresh().await;
    assert_eq!(call(&state, "PUT", "/attrv", vec![], b"").await.status, 200);
    set_versioning(&state, "attrv", "Enabled").await;
    let first = version_header(&put(&state, "/attrv/k", b"one").await).expect("an id");
    put(&state, "/attrv/k", b"longer").await;
    let named = attributes(
        &state,
        "/attrv/k",
        vec![("versionId", Some(&first))],
        vec![("x-amz-object-attributes", "ObjectSize")],
    )
    .await;
    assert!(
        named.body.contains("<ObjectSize>3</ObjectSize>"),
        "{}",
        named.body
    );
    assert_eq!(header(&named, "x-amz-version-id"), Some(first.as_str()));
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn attributes_need_both_the_attributes_and_the_read_permission() {
    let _why = IGNORED;
    let (state, _pool) = fresh().await;
    let ann = user_with_key(state.storage(), "ann", "attrspace", Role::Member, true).await;
    let as_ann = |method: &'static str,
                  path: &'static str,
                  query: Vec<(&'static str, Option<&'static str>)>| {
        call_as(
            &state,
            ann.pair(),
            method,
            path,
            query,
            vec![("x-amz-object-attributes", ALL)],
            b"",
        )
    };
    assert_eq!(as_ann("PUT", "/attrd", vec![]).await.status, 200);
    assert_eq!(as_ann("PUT", "/attrd/k", vec![]).await.status, 200);
    for action in ["s3:GetObjectAttributes", "s3:GetObject"] {
        let deny = format!(
            r#"{{"Version":"2012-10-17","Statement":[{{"Effect":"Deny","Principal":{{"AWS":"arn:aws:iam::attrspace:user/ann"}},"Action":"{action}","Resource":"arn:aws:s3:::attrd/*"}}]}}"#
        );
        let set = put_policy(&state, "attrd", &deny).await;
        assert!(set.status == 200 || set.status == 204, "{}", set.body);
        let refused = as_ann("GET", "/attrd/k", vec![("attributes", None)]).await;
        assert_eq!(
            refused.code.as_deref(),
            Some("AccessDenied"),
            "{action}: {}",
            refused.body
        );
    }
    assert_eq!(
        call(&state, "DELETE", "/attrd", vec![("policy", None)], b"")
            .await
            .status,
        204
    );
    let allowed = as_ann("GET", "/attrd/k", vec![("attributes", None)]).await;
    assert_eq!(allowed.status, 200, "{}", allowed.body);
}
