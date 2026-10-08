//! The low-privilege probe, against a real node: a member of the bucket's own space with no grant calls EVERY
//! implemented S3 operation on someone else's bucket and is refused each time; the bucket's creator, sending the same
//! requests, is refused none of them — so no refusal below comes from the request rather than from the decision.

use std::collections::BTreeSet;

use tessari_s3_core::authz::Role;
use tessari_s3_core::dispatch::{
    DispatchRequest, Method, Operation, Target, dispatch, implemented,
};

use crate::access::{Key, user_with_key};
use crate::{IGNORED, call_as, fresh};

struct Probe {
    operation: Operation,
    method: &'static str,
    path: String,
    query: Vec<(&'static str, Option<String>)>,
    headers: Vec<(&'static str, &'static str)>,
    body: &'static [u8],
}

fn probe(
    operation: Operation,
    method: &'static str,
    path: &str,
    query: &[(&'static str, Option<&str>)],
) -> Probe {
    Probe {
        operation,
        method,
        path: path.to_owned(),
        query: query
            .iter()
            .map(|(name, value)| (*name, value.map(str::to_owned)))
            .collect(),
        headers: vec![],
        body: b"",
    }
}

const DELETE_BODY: &[u8] = b"<Delete><Object><Key>a.txt</Key></Object></Delete>";
const COMPLETE_BODY: &[u8] =
    b"<CompleteMultipartUpload><Part><PartNumber>1</PartNumber><ETag>\"x\"</ETag></Part></CompleteMultipartUpload>";

/// Every implemented operation, addressed at `ann-photos`; destructive ones last, so the control arm reaches each.
fn table(upload: &str) -> Vec<Probe> {
    let id = Some(upload);
    let part = [("partNumber", Some("1")), ("uploadId", id)];
    let copy = ("x-amz-copy-source", "/ann-photos/a.txt");
    vec![
        probe(Operation::ListBuckets, "GET", "/", &[]),
        probe(Operation::CreateBucket, "PUT", "/another-bucket", &[]),
        probe(Operation::HeadBucket, "HEAD", "/ann-photos", &[]),
        probe(
            Operation::GetBucketLocation,
            "GET",
            "/ann-photos",
            &[("location", None)],
        ),
        probe(Operation::ListObjects, "GET", "/ann-photos", &[]),
        probe(
            Operation::ListObjectsV2,
            "GET",
            "/ann-photos",
            &[("list-type", Some("2"))],
        ),
        probe(
            Operation::ListMultipartUploads,
            "GET",
            "/ann-photos",
            &[("uploads", None)],
        ),
        probe(
            Operation::ListParts,
            "GET",
            "/ann-photos/big",
            &[("uploadId", id)],
        ),
        probe(Operation::GetObject, "GET", "/ann-photos/a.txt", &[]),
        probe(Operation::HeadObject, "HEAD", "/ann-photos/a.txt", &[]),
        probe(
            Operation::GetObjectTagging,
            "GET",
            "/ann-photos/a.txt",
            &[("tagging", None)],
        ),
        Probe {
            body: b"new",
            ..probe(Operation::PutObject, "PUT", "/ann-photos/b.txt", &[])
        },
        Probe {
            headers: vec![copy],
            ..probe(Operation::CopyObject, "PUT", "/ann-photos/c.txt", &[])
        },
        probe(
            Operation::CreateMultipartUpload,
            "POST",
            "/ann-photos/other",
            &[("uploads", None)],
        ),
        Probe {
            body: b"part",
            ..probe(Operation::UploadPart, "PUT", "/ann-photos/big", &part)
        },
        Probe {
            headers: vec![copy],
            ..probe(Operation::UploadPartCopy, "PUT", "/ann-photos/big", &part)
        },
        Probe {
            body: COMPLETE_BODY,
            ..probe(
                Operation::CompleteMultipartUpload,
                "POST",
                "/ann-photos/big",
                &[("uploadId", id)],
            )
        },
        Probe {
            body: DELETE_BODY,
            ..probe(
                Operation::DeleteObjects,
                "POST",
                "/ann-photos",
                &[("delete", None)],
            )
        },
        probe(Operation::DeleteObject, "DELETE", "/ann-photos/a.txt", &[]),
        probe(
            Operation::AbortMultipartUpload,
            "DELETE",
            "/ann-photos/big",
            &[("uploadId", id)],
        ),
        probe(Operation::DeleteBucket, "DELETE", "/ann-photos", &[]),
    ]
}

/// Whether the dispatcher reads `row` as the operation it names.
fn addresses(row: &Probe) -> bool {
    let method = match row.method {
        "GET" => Method::Get,
        "HEAD" => Method::Head,
        "PUT" => Method::Put,
        "POST" => Method::Post,
        _ => Method::Delete,
    };
    let target = match row.path.matches('/').count() {
        _ if row.path == "/" => Target::Service,
        1 => Target::Bucket,
        _ => Target::Object,
    };
    let query: Vec<(&str, Option<&str>)> = row
        .query
        .iter()
        .map(|(name, value)| (*name, value.as_deref()))
        .collect();
    let names: Vec<&str> = row.headers.iter().map(|(name, _)| *name).collect();
    dispatch(&DispatchRequest {
        method,
        target,
        query: &query,
        header_names: &names,
    })
    .is_ok_and(|spec| spec.operation == row.operation)
}

async fn send(state: &tessari_s3_api::ApiState, key: &Key, row: &Probe) -> crate::Seen {
    let query: Vec<(&str, Option<&str>)> = row
        .query
        .iter()
        .map(|(name, value)| (*name, value.as_deref()))
        .collect();
    call_as(
        state,
        key.pair(),
        row.method,
        &row.path,
        query,
        row.headers.clone(),
        row.body,
    )
    .await
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_member_without_a_grant_is_refused_every_operation_its_creator_is_not() {
    let _why = IGNORED;
    let (state, _) = fresh().await;
    let storage = state.storage();
    let ann = user_with_key(storage, "ann", "alpha", Role::Member, true).await;
    let cid = user_with_key(storage, "cid", "alpha", Role::Member, false).await;
    for (method, path, body) in [
        ("PUT", "/ann-photos", &b""[..]),
        ("PUT", "/ann-photos/a.txt", b"a"),
    ] {
        assert_eq!(
            call_as(&state, ann.pair(), method, path, vec![], vec![], body)
                .await
                .status,
            200
        );
    }
    let started = call_as(
        &state,
        ann.pair(),
        "POST",
        "/ann-photos/big",
        vec![("uploads", None)],
        vec![],
        b"",
    )
    .await;
    let upload = started
        .body
        .split("<UploadId>")
        .nth(1)
        .and_then(|rest| rest.split("</UploadId>").next())
        .expect("an upload id")
        .to_owned();
    let rows = table(&upload);

    let probed: BTreeSet<String> = rows
        .iter()
        .map(|row| format!("{:?}", row.operation))
        .collect();
    let all: BTreeSet<String> = implemented().iter().map(|op| format!("{op:?}")).collect();
    assert_eq!(probed, all, "every implemented operation is probed");
    for row in &rows {
        assert!(
            addresses(row),
            "{:?}: the request addresses it",
            row.operation
        );
    }

    for row in &rows {
        let answer = send(&state, &cid, row).await;
        if row.operation == Operation::ListBuckets {
            assert_eq!(answer.status, 200);
            assert!(
                !answer.body.contains("ann-photos"),
                "listed only what it may see"
            );
            continue;
        }
        assert_eq!(answer.status, 403, "{:?}: {}", row.operation, answer.body);
        if row.method != "HEAD" {
            assert_eq!(
                answer.code.as_deref(),
                Some("AccessDenied"),
                "{:?}",
                row.operation
            );
        }
    }

    for row in &rows {
        let answer = send(&state, &ann, row).await;
        assert_ne!(
            answer.status, 403,
            "the control: the creator is not refused {:?} ({})",
            row.operation, answer.body
        );
    }
}
