//! Dispatch over the whole S3 model: every row resolves to itself (or, for a shadowed row, to the operation that
//! answers its wire shape), shared method+path groups resolve by the published precedence, and nothing unknown
//! reaches a data operation.

#![cfg(test)]

use tessari_s3_core::dispatch::{
    CATALOG, DispatchError, DispatchRequest, Method, OperationSpec, Target, dispatch,
    is_implemented,
};
use tessari_s3_types::ErrorCode;

fn resolve(
    method: Method,
    target: Target,
    query: &[(&str, Option<&str>)],
    headers: &[&str],
) -> Result<&'static str, DispatchError> {
    dispatch(&DispatchRequest {
        method,
        target,
        query,
        header_names: headers,
    })
    .map(|spec| spec.name)
}

/// The smallest request that carries everything `spec` requires.
fn canonical(
    spec: &OperationSpec,
) -> (Vec<(&'static str, Option<&'static str>)>, Vec<&'static str>) {
    let mut query: Vec<(&str, Option<&str>)> = spec.discriminators.to_vec();
    query.extend(spec.required_query.iter().map(|name| (*name, Some("1"))));
    (query, spec.required_headers.to_vec())
}

/// Rows whose wire shape another operation answers, and that operation.
const SHADOWED: [(&str, &str); 5] = [
    ("GetBucketLifecycle", "GetBucketLifecycleConfiguration"),
    (
        "GetBucketNotification",
        "GetBucketNotificationConfiguration",
    ),
    ("PutBucketLifecycle", "PutBucketLifecycleConfiguration"),
    (
        "PutBucketNotification",
        "PutBucketNotificationConfiguration",
    ),
    ("ListDirectoryBuckets", "ListBuckets"),
];

#[test]
fn the_catalog_is_the_116_operations_of_the_model() {
    assert_eq!(CATALOG.len(), 116);
    let mut names: Vec<&str> = CATALOG.iter().map(|spec| spec.name).collect();
    names.dedup();
    assert_eq!(names.len(), 116, "names are unique and sorted");
}

#[test]
fn every_row_resolves_to_its_own_operation_or_the_one_that_answers_its_shape() {
    let mut failures = Vec::new();
    for spec in &CATALOG {
        let (query, headers) = canonical(spec);
        let expected = SHADOWED
            .iter()
            .find(|(name, _)| *name == spec.name)
            .map_or(spec.name, |(_, twin)| *twin);
        let got = resolve(spec.method, spec.target, &query, &headers);
        if got != Ok(expected) {
            failures.push(format!("{}: expected {expected}, got {got:?}", spec.name));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of 116 rows misroute:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn put_on_an_object_follows_the_published_precedence() {
    let (copy, upload, part) = (
        ("partNumber", Some("1")),
        ("uploadId", Some("u")),
        "x-amz-copy-source",
    );
    let put = |query: &[(&str, Option<&str>)], headers: &[&str]| {
        resolve(Method::Put, Target::Object, query, headers)
    };
    assert_eq!(put(&[copy, upload], &[part]), Ok("UploadPartCopy"));
    assert_eq!(put(&[copy, upload], &[]), Ok("UploadPart"));
    assert_eq!(put(&[("acl", None)], &[]), Ok("PutObjectAcl"));
    assert_eq!(put(&[("tagging", None)], &[part]), Ok("PutObjectTagging"));
    assert_eq!(
        put(&[], &["X-Amz-Copy-Source"]),
        Ok("CopyObject"),
        "header names match in any case"
    );
    assert_eq!(put(&[], &[]), Ok("PutObject"));
}

#[test]
fn get_on_a_bucket_tells_the_list_operations_apart() {
    let get = |query: &[(&str, Option<&str>)]| resolve(Method::Get, Target::Bucket, query, &[]);
    assert_eq!(
        get(&[("list-type", Some("2")), ("prefix", Some("a"))]),
        Ok("ListObjectsV2")
    );
    assert_eq!(get(&[("versions", None)]), Ok("ListObjectVersions"));
    assert_eq!(get(&[("uploads", None)]), Ok("ListMultipartUploads"));
    assert_eq!(
        get(&[("prefix", Some("a")), ("max-keys", Some("2"))]),
        Ok("ListObjects")
    );
    assert_eq!(
        get(&[("list-type", Some("1"))]),
        Err(DispatchError::UnknownQueryParameter {
            name: "list-type".to_owned()
        }),
        "a list-type other than 2 is not ListObjects V1 in disguise"
    );
}

#[test]
fn a_subresource_is_matched_by_name_whatever_its_value() {
    for value in [None, Some(""), Some("x")] {
        assert_eq!(
            resolve(Method::Get, Target::Object, &[("acl", value)], &[]),
            Ok("GetObjectAcl"),
            "{value:?}"
        );
    }
}

#[test]
fn an_unknown_or_misplaced_subresource_never_reaches_put_object() {
    let put = |query: &[(&str, Option<&str>)]| resolve(Method::Put, Target::Object, query, &[]);
    assert_eq!(
        put(&[("cors", None)]),
        Err(DispatchError::MethodNotAllowed),
        "cors is a bucket subresource"
    );
    assert_eq!(
        put(&[("bogus", None)]),
        Err(DispatchError::UnknownQueryParameter {
            name: "bogus".to_owned()
        })
    );
    assert_eq!(
        put(&[("partNumber", Some("1"))]),
        Err(DispatchError::UnknownQueryParameter {
            name: "partNumber".to_owned()
        }),
        "a part number without an upload id is not a PutObject"
    );
}

#[test]
fn a_selected_operation_missing_a_required_header_is_refused() {
    let refusal = resolve(Method::Get, Target::Object, &[("attributes", None)], &[]);
    let expected = DispatchError::MissingParameter {
        operation: "GetObjectAttributes",
        missing: "x-amz-object-attributes",
    };
    assert_eq!(refusal, Err(expected));
}

#[test]
fn a_method_the_model_never_uses_on_a_target_is_refused() {
    assert_eq!(
        resolve(Method::Delete, Target::Service, &[], &[]),
        Err(DispatchError::MethodNotAllowed)
    );
    assert_eq!(
        resolve(Method::Post, Target::Object, &[("acl", None)], &[]),
        Err(DispatchError::MethodNotAllowed)
    );
}

#[test]
fn presign_parameters_and_the_sdk_operation_hint_are_not_unknown_parameters() {
    let query = [
        ("X-Amz-Algorithm", Some("AWS4-HMAC-SHA256")),
        ("X-Amz-Credential", Some("c")),
        ("X-Amz-Date", Some("d")),
        ("X-Amz-Expires", Some("1")),
        ("X-Amz-SignedHeaders", Some("host")),
        ("X-Amz-Signature", Some("s")),
        ("x-id", Some("GetObject")),
        ("versionId", Some("v")),
    ];
    assert_eq!(
        resolve(Method::Get, Target::Object, &query, &[]),
        Ok("GetObject")
    );
}

#[test]
fn a_fixed_path_operation_resolves_only_on_its_path() {
    let headers = ["x-amz-request-route", "x-amz-request-token"];
    assert_eq!(
        resolve(
            Method::Post,
            Target::Named("/WriteGetObjectResponse"),
            &[],
            &headers
        ),
        Ok("WriteGetObjectResponse")
    );
    assert_eq!(
        resolve(Method::Post, Target::Named("/Other"), &[], &headers),
        Err(DispatchError::MethodNotAllowed)
    );
}

#[test]
fn exactly_the_operations_with_handlers_are_implemented() {
    let implemented: Vec<&str> = CATALOG
        .iter()
        .filter(|spec| is_implemented(spec.operation))
        .map(|spec| spec.name)
        .collect();
    let expected = [
        "AbortMultipartUpload",
        "CompleteMultipartUpload",
        "CopyObject",
        "CreateBucket",
        "CreateMultipartUpload",
        "DeleteBucket",
        "DeleteObject",
        "DeleteObjects",
        "GetBucketLocation",
        "GetObject",
        "GetObjectTagging",
        "HeadBucket",
        "HeadObject",
        "ListBuckets",
        "ListMultipartUploads",
        "ListObjects",
        "ListObjectsV2",
        "ListParts",
        "PutObject",
        "UploadPart",
        "UploadPartCopy",
    ];
    assert_eq!(implemented, expected);
}

#[test]
fn every_refusal_carries_a_catalog_code() {
    let unknown = DispatchError::UnknownQueryParameter {
        name: String::new(),
    };
    let missing = DispatchError::MissingParameter {
        operation: "",
        missing: "",
    };
    assert_eq!(
        DispatchError::MethodNotAllowed.s3_code(),
        ErrorCode::MethodNotAllowed
    );
    assert_eq!(unknown.s3_code(), ErrorCode::InvalidArgument);
    assert_eq!(missing.s3_code(), ErrorCode::InvalidRequest);
}
