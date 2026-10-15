//! Bucket quotas enforced, against a real node: on a bucket at its limit every operation that adds to it is refused
//! with `InvalidRequest` and every other write is not; raising the limit lets the same requests through; an overwrite
//! is not a new object. The table of writes is the catalog's, so an operation added later must be classified here.

use std::collections::BTreeSet;

use tessari_s3_api::ApiState;
use tessari_s3_core::authz::{Need, required};
use tessari_s3_core::dispatch::{Operation, implemented};
use tessari_s3_core::quota::{Quota, adds_to_bucket};
use tessari_s3_types::BucketName;

use crate::delete_many::md5_of;
use crate::{IGNORED, Seen, call, call_with, fresh};

struct Write {
    operation: Operation,
    method: &'static str,
    path: &'static str,
    query: Vec<(&'static str, Option<String>)>,
    headers: Vec<(&'static str, String)>,
    body: Vec<u8>,
}

fn write(operation: Operation, method: &'static str, path: &'static str) -> Write {
    Write {
        operation,
        method,
        path,
        query: Vec::new(),
        headers: Vec::new(),
        body: Vec::new(),
    }
}

const DELETE: &str = "<Delete><Object><Key>zzz</Key></Object></Delete>";

fn source() -> Vec<(&'static str, String)> {
    vec![("x-amz-copy-source", "/source/src".to_owned())]
}

/// Every implemented operation that writes an object, against bucket `full`: its open upload `upload` (with part 1
/// tagged `first_part`) and a second one, `spare`, for the abort.
fn writes(upload: &str, first_part: &str, spare: &str) -> Vec<Write> {
    let id = Some(upload.to_owned());
    let part = |number: &str| {
        vec![
            ("partNumber", Some(number.to_owned())),
            ("uploadId", id.clone()),
        ]
    };
    let complete = format!(
        "<CompleteMultipartUpload><Part><PartNumber>1</PartNumber><ETag>{first_part}</ETag></Part>\
         </CompleteMultipartUpload>"
    );
    vec![
        Write {
            body: b"x".to_vec(),
            ..write(Operation::PutObject, "PUT", "/full/b")
        },
        Write {
            headers: source(),
            ..write(Operation::CopyObject, "PUT", "/full/c")
        },
        Write {
            query: part("2"),
            body: b"x".to_vec(),
            ..write(Operation::UploadPart, "PUT", "/full/big")
        },
        Write {
            query: part("3"),
            headers: source(),
            ..write(Operation::UploadPartCopy, "PUT", "/full/big")
        },
        Write {
            query: vec![("uploadId", id.clone())],
            body: complete.into_bytes(),
            ..write(Operation::CompleteMultipartUpload, "POST", "/full/big")
        },
        Write {
            query: vec![("uploads", None)],
            ..write(Operation::CreateMultipartUpload, "POST", "/full/other")
        },
        Write {
            query: vec![("delete", None)],
            headers: vec![("content-md5", md5_of(DELETE))],
            body: DELETE.as_bytes().to_vec(),
            ..write(Operation::DeleteObjects, "POST", "/full")
        },
        write(Operation::DeleteObject, "DELETE", "/full/zzz"),
        Write {
            query: vec![("uploadId", Some(spare.to_owned()))],
            ..write(Operation::AbortMultipartUpload, "DELETE", "/full/spare")
        },
    ]
}

async fn send(state: &ApiState, row: &Write) -> Seen {
    let query = row
        .query
        .iter()
        .map(|(name, value)| (*name, value.as_deref()))
        .collect();
    let headers = row
        .headers
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect();
    call_with(state, row.method, row.path, query, headers, &row.body).await
}

fn between<'a>(text: &'a str, open: &str, close: &str) -> &'a str {
    text.split(open)
        .nth(1)
        .and_then(|rest| rest.split(close).next())
        .expect("the element")
}

async fn limit(state: &ApiState, bucket: &str, quota: Quota) {
    let name = BucketName::new(bucket).expect("a name");
    assert!(
        state
            .storage()
            .buckets()
            .set_quota(&name, quota)
            .await
            .expect("set")
    );
}

async fn measure(state: &ApiState) {
    state
        .storage()
        .usage()
        .refresh("standalone")
        .await
        .expect("measured");
}

async fn start(state: &ApiState, path: &str) -> String {
    let started = call(state, "POST", path, vec![("uploads", None)], b"").await;
    assert_eq!(started.status, 200, "{}", started.body);
    between(&started.body, "<UploadId>", "</UploadId>").to_owned()
}

async fn ok(state: &ApiState, method: &str, path: &str, body: &[u8]) {
    let seen = call(state, method, path, vec![], body).await;
    assert_eq!(seen.status, 200, "{method} {path}: {}", seen.body);
}

const fn bytes(max: u64) -> Quota {
    Quota {
        max_bytes: Some(max),
        max_objects: None,
    }
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_full_bucket_refuses_every_adding_write_and_no_other() {
    let _why = IGNORED;
    let (state, _) = fresh().await;
    ok(&state, "PUT", "/full", b"").await;
    ok(&state, "PUT", "/source", b"").await;
    ok(&state, "PUT", "/source/src", b"12345").await;
    let upload = start(&state, "/full/big").await;
    let spare = start(&state, "/full/spare").await;
    let first = call(
        &state,
        "PUT",
        "/full/big",
        vec![("partNumber", Some("1")), ("uploadId", Some(&upload))],
        b"abc",
    )
    .await;
    assert_eq!(first.status, 200);
    let first_part = first.headers["etag"].to_str().expect("etag").to_owned();
    ok(&state, "PUT", "/full/a", b"0123456789").await;
    limit(&state, "full", bytes(10)).await;
    measure(&state).await;

    let rows = writes(&upload, &first_part, &spare);
    let tabled: BTreeSet<String> = rows
        .iter()
        .map(|row| format!("{:?}", row.operation))
        .collect();
    let catalog: BTreeSet<String> = implemented()
        .iter()
        .filter(|op| required(**op).is_some_and(|needs| needs.primary == Need::WriteObject))
        .map(|op| format!("{op:?}"))
        .collect();
    assert_eq!(tabled, catalog, "every implemented write is classified");

    let (adding, others): (Vec<&Write>, Vec<&Write>) =
        rows.iter().partition(|row| adds_to_bucket(row.operation));
    for row in &adding {
        let seen = send(&state, row).await;
        assert_eq!(
            (seen.status, seen.code.as_deref()),
            (400, Some("InvalidRequest")),
            "{:?}: {}",
            row.operation,
            seen.body
        );
        assert!(
            seen.body.contains("bucket quota exceeded"),
            "{:?}",
            row.operation
        );
    }
    // The writes that add nothing are answered as on a bucket with no limit.
    for row in &others {
        let seen = send(&state, row).await;
        assert!(
            (200..300).contains(&seen.status),
            "{:?}: {} {}",
            row.operation,
            seen.status,
            seen.body
        );
    }
    // Control: the limit, not the request, refused them — with room, the same requests succeed.
    limit(&state, "full", bytes(1_000)).await;
    for row in &adding {
        let seen = send(&state, row).await;
        assert!(
            (200..300).contains(&seen.status),
            "{:?}: {} {}",
            row.operation,
            seen.status,
            seen.body
        );
    }
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn an_object_limit_refuses_a_new_key_but_not_an_overwrite() {
    let _why = IGNORED;
    let (state, _) = fresh().await;
    ok(&state, "PUT", "/counted", b"").await;
    ok(&state, "PUT", "/counted/a", b"one").await;
    limit(
        &state,
        "counted",
        Quota {
            max_bytes: None,
            max_objects: Some(1),
        },
    )
    .await;
    measure(&state).await;
    ok(&state, "PUT", "/counted/a", b"replaced").await;
    let refused = call(&state, "PUT", "/counted/b", vec![], b"two").await;
    assert_eq!(
        (refused.status, refused.code.as_deref()),
        (400, Some("InvalidRequest")),
        "{}",
        refused.body
    );
    // A bucket with no limit is not measured against anything.
    ok(&state, "PUT", "/free", b"").await;
    ok(&state, "PUT", "/free/b", b"two").await;
}
