//! CreateBucket, HeadBucket, ListBuckets, GetBucketLocation and DeleteBucket against a real node.

use tessari_s3_infrastructure::tessaridb::Value;

use crate::{IGNORED, call, fresh};

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_bucket_lives_from_create_to_delete() {
    let _why = IGNORED;
    let (state, _) = fresh().await;
    let created = call(&state, "PUT", "/photos", vec![], b"").await;
    assert_eq!(created.status, 200, "{}", created.body);
    assert_eq!(
        created
            .headers
            .get("location")
            .and_then(|v| v.to_str().ok()),
        Some("/photos")
    );
    let again = call(&state, "PUT", "/photos", vec![], b"").await;
    assert_eq!(
        (again.status, again.code.as_deref()),
        (409, Some("BucketAlreadyOwnedByYou"))
    );
    let head = call(&state, "HEAD", "/photos", vec![], b"").await;
    assert_eq!(head.status, 200);
    assert_eq!(
        head.headers
            .get("x-amz-bucket-region")
            .and_then(|v| v.to_str().ok()),
        Some("us-east-1")
    );
    let location = call(&state, "GET", "/photos", vec![("location", None)], b"").await;
    assert_eq!(location.status, 200);
    assert!(
        location.body.contains("></LocationConstraint>"),
        "us-east-1 is the empty constraint: {}",
        location.body
    );
    let listed = call(&state, "GET", "/", vec![], b"").await;
    assert!(
        listed.body.contains("<Name>photos</Name>"),
        "{}",
        listed.body
    );
    assert!(listed.body.contains("<CreationDate>20"), "{}", listed.body);
    let deleted = call(&state, "DELETE", "/photos", vec![], b"").await;
    assert_eq!(deleted.status, 204, "{}", deleted.body);
    let gone = call(&state, "HEAD", "/photos", vec![], b"").await;
    assert_eq!((gone.status, gone.body.as_str()), (404, ""));
    let twice = call(&state, "DELETE", "/photos", vec![], b"").await;
    assert_eq!(
        (twice.status, twice.code.as_deref()),
        (404, Some("NoSuchBucket"))
    );
    let empty = call(&state, "GET", "/", vec![], b"").await;
    assert!(!empty.body.contains("<Bucket>"), "{}", empty.body);
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_location_constraint_must_name_this_servers_region() {
    let (state, _) = fresh().await;
    let other = b"<CreateBucketConfiguration><LocationConstraint>eu-west-1</LocationConstraint></CreateBucketConfiguration>";
    let refused = call(&state, "PUT", "/elsewhere", vec![], other).await;
    assert_eq!(
        (refused.status, refused.code.as_deref()),
        (400, Some("IllegalLocationConstraintException"))
    );
    let own = b"<CreateBucketConfiguration><LocationConstraint>us-east-1</LocationConstraint></CreateBucketConfiguration>";
    assert_eq!(call(&state, "PUT", "/here", vec![], own).await.status, 200);
    let hostile =
        b"<!DOCTYPE d [<!ENTITY x SYSTEM \"file:///etc/hosts\">]><CreateBucketConfiguration/>";
    let malformed = call(&state, "PUT", "/hostile", vec![], hostile).await;
    assert_eq!(
        (malformed.status, malformed.code.as_deref()),
        (400, Some("MalformedXML"))
    );
    let listed = call(&state, "GET", "/", vec![], b"").await;
    assert!(
        !listed.body.contains("<Name>elsewhere</Name>")
            && !listed.body.contains("<Name>hostile</Name>")
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_bucket_holding_an_object_is_not_deleted() {
    let (state, planter) = fresh().await;
    assert_eq!(call(&state, "PUT", "/full", vec![], b"").await.status, 200);
    assert_eq!(
        call(&state, "PUT", "/full-neighbour", vec![], b"")
            .await
            .status,
        200
    );
    // An object in the NEIGHBOUR's range must not count against "full": the range check reads `bucket`, not a prefix.
    let plant = "CREATE objects:$id = { bucket_name: $bucket, key: 'k', incarnation: rand::uuid(), size: 0, etag: 'e', \
                 modified: time::now(), headers: {}, metadata: {}, checksums: {} };";
    let neighbour = vec![
        (
            "id".to_owned(),
            Value::String("full-neighbour/k".to_owned()),
        ),
        (
            "bucket".to_owned(),
            Value::String("full-neighbour".to_owned()),
        ),
    ];
    planter.run(plant, neighbour).await.expect("planted");
    assert_eq!(
        call(&state, "DELETE", "/full", vec![], b"").await.status,
        204,
        "an empty bucket deletes"
    );
    assert_eq!(call(&state, "PUT", "/full", vec![], b"").await.status, 200);
    let own = vec![
        ("id".to_owned(), Value::String("full/k".to_owned())),
        ("bucket".to_owned(), Value::String("full".to_owned())),
    ];
    planter.run(plant, own).await.expect("planted");
    let kept = call(&state, "DELETE", "/full", vec![], b"").await;
    assert_eq!(
        (kept.status, kept.code.as_deref()),
        (409, Some("BucketNotEmpty"))
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn list_buckets_pages_in_byte_order_with_a_token() {
    let (state, _) = fresh().await;
    for name in ["ccc", "aaa", "bbb"] {
        assert_eq!(
            call(&state, "PUT", &format!("/{name}"), vec![], b"")
                .await
                .status,
            200
        );
    }
    let first = call(&state, "GET", "/", vec![("max-buckets", Some("2"))], b"").await;
    let names = |body: &str| {
        body.split("<Name>")
            .skip(1)
            .map(|r| r.split('<').next().unwrap_or("").to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(names(&first.body), ["aaa", "bbb"]);
    let token = first
        .body
        .split("<ContinuationToken>")
        .nth(1)
        .and_then(|r| r.split('<').next())
        .expect("a token")
        .to_owned();
    let second = call(
        &state,
        "GET",
        "/",
        vec![
            ("max-buckets", Some("2")),
            ("continuation-token", Some(&token)),
        ],
        b"",
    )
    .await;
    assert_eq!(names(&second.body), ["ccc"]);
    assert!(
        !second.body.contains("<ContinuationToken>"),
        "the last page carries no token"
    );
    let prefixed = call(&state, "GET", "/", vec![("prefix", Some("b"))], b"").await;
    assert_eq!(names(&prefixed.body), ["bbb"]);
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_body_that_differs_from_its_signed_hash_is_bad_digest_and_creates_nothing() {
    use crate::signer::{Unsigned, sign_with_body};
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;
    let (state, _) = fresh().await;
    let declared = b"<CreateBucketConfiguration/>";
    let amz_date = crate::amz_now_for_tests();
    let unsigned = Unsigned {
        method: "PUT",
        host: "localhost:9100",
        path: "/tampered",
        query: vec![],
        headers: vec![],
        access_key: crate::ACCESS_KEY,
        secret: crate::SECRET,
        region: "us-east-1",
        amz_date: &amz_date,
    };
    let (target, headers) = sign_with_body(&unsigned, declared);
    let mut builder = Request::builder().method("PUT").uri(target);
    for (name, value) in &headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    let sent = builder
        .body(Body::from(&b"<CreateBucketConfiguration />"[..]))
        .expect("request");
    let response = tessari_s3_api::router(state.clone())
        .oneshot(sent)
        .await
        .expect("router");
    assert_eq!(response.status().as_u16(), 400);
    let head = call(&state, "HEAD", "/tampered", vec![], b"").await;
    assert_eq!(head.status, 404, "nothing was created");
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_quota_is_kept_on_the_bucket_and_cleared_by_setting_none() {
    use tessari_s3_core::quota::Quota;
    use tessari_s3_types::BucketName;
    let _why = IGNORED;
    let (state, _) = fresh().await;
    assert_eq!(
        call(&state, "PUT", "/limited", vec![], b"").await.status,
        200
    );
    let buckets = state.storage().buckets();
    let name = BucketName::new("limited").expect("a name");
    let read = || async {
        buckets
            .get(&name)
            .await
            .expect("read")
            .expect("bucket")
            .quota
    };
    assert_eq!(read().await, Quota::default(), "a new bucket has no limit");
    let limit = Quota {
        max_bytes: Some(1_000),
        max_objects: Some(3),
    };
    assert!(buckets.set_quota(&name, limit).await.expect("set"));
    assert_eq!(read().await, limit);
    let bytes_only = Quota {
        max_bytes: Some(5),
        max_objects: None,
    };
    assert!(buckets.set_quota(&name, bytes_only).await.expect("set"));
    assert_eq!(read().await, bytes_only, "None removes that limit");
    let nowhere = BucketName::new("nowhere").expect("a name");
    assert!(!buckets.set_quota(&nowhere, limit).await.expect("answered"));
}
