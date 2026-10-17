//! The low-privilege probe for bucket policies (G019 C1): the bucket's creator — whom its grants allow everything —
//! calls EVERY implemented operation on a bucket whose policy denies `s3:*` and is refused each time; with the policy
//! removed, the same requests are refused none of them, so every refusal below comes from the policy.

use std::collections::BTreeSet;

use tessari_s3_core::dispatch::{Operation, implemented};

use crate::access_probe::{addresses, prepared, send, table};
use crate::policy::put_policy;
use crate::{IGNORED, call, fresh};

/// Operations no bucket policy governs: listing every bucket, and creating one that does not exist yet.
const UNGOVERNED: [Operation; 2] = [Operation::ListBuckets, Operation::CreateBucket];

const DENY_ALL: &str = r#"{"Version":"2012-10-17","Statement":[{"Sid":"nothing","Effect":"Deny","Principal":"*","Action":"s3:*",
"Resource":["arn:aws:s3:::ann-photos","arn:aws:s3:::ann-photos/*"]}]}"#;

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_deny_all_policy_refuses_the_creator_every_operation_on_every_path() {
    let _why = IGNORED;
    let (state, _) = fresh().await;
    let (ann, _cid, upload) = prepared(&state).await;
    let rows = table(&upload);
    let probed: BTreeSet<String> = rows
        .iter()
        .map(|row| format!("{:?}", row.operation))
        .collect();
    let all: BTreeSet<String> = implemented().iter().map(|op| format!("{op:?}")).collect();
    assert_eq!(probed, all, "every implemented operation is probed");
    assert!(
        rows.iter().all(addresses),
        "each row addresses its operation"
    );

    assert_eq!(put_policy(&state, "ann-photos", DENY_ALL).await.status, 204);
    let mut refused = 0_usize;
    for row in rows
        .iter()
        .filter(|row| !UNGOVERNED.contains(&row.operation))
    {
        let answer = send(&state, &ann, row).await;
        if row.operation == Operation::DeleteObjects {
            // Each key of a batch is answered on its own.
            assert_eq!(answer.status, 200, "{}", answer.body);
            assert!(
                answer.body.contains("<Code>AccessDenied</Code>"),
                "{}",
                answer.body
            );
        } else {
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
        refused = refused.saturating_add(1);
    }
    let governed = all.len().saturating_sub(UNGOVERNED.len());
    println!(
        "paths: source={} | enforced={refused} | verified={refused} | exempt=[ListBuckets, CreateBucket]",
        all.len()
    );
    assert_eq!(refused, governed);

    // The control: Root lifts the policy, and the same requests are refused none of them.
    assert_eq!(
        call(&state, "DELETE", "/ann-photos", vec![("policy", None)], b"")
            .await
            .status,
        204
    );
    for row in rows
        .iter()
        .filter(|row| !UNGOVERNED.contains(&row.operation))
    {
        if matches!(row.operation, Operation::PutBucketPolicy) {
            continue; // it would put a policy back for the rows after it; its own control is in `policy`
        }
        let answer = send(&state, &ann, row).await;
        assert_ne!(
            answer.status, 403,
            "the control: {:?} ({})",
            row.operation, answer.body
        );
        if row.operation == Operation::DeleteObjects {
            assert!(!answer.body.contains("AccessDenied"), "{}", answer.body);
        }
    }
}

/// An operator — every role and grant allows it everything — with a key.
async fn operator(storage: &tessari_s3_storage::Storage) -> crate::access::Key {
    use tessari_s3_core::authz::{Role, UserName};
    use tessari_s3_storage::users::NewUser;
    let name = UserName::new("ops").expect("a user name");
    storage
        .users()
        .create(&NewUser {
            name: name.clone(),
            space: crate::access::space("default"),
            role: Role::Member,
            create_buckets: true,
            operator: true,
            cluster_viewer: true,
        })
        .await
        .expect("created");
    let issued = storage
        .users()
        .issue_key(&name)
        .await
        .expect("issued")
        .expect("exists");
    crate::access::Key {
        id: issued.access_key_id,
        secret: issued.secret.expose().to_owned(),
    }
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_deny_all_policy_refuses_an_operator_every_console_path_on_the_bucket() {
    let _why = IGNORED;
    let (state, console) = crate::fresh_console().await;
    let ops = operator(state.storage()).await;
    assert_eq!(call(&state, "PUT", "/held", vec![], b"").await.status, 200);
    assert_eq!(
        call(&state, "PUT", "/held/a.txt", vec![], b"a")
            .await
            .status,
        200
    );
    let token = crate::console_users::sign_in_as(&console, &ops)
        .await
        .expect("signed in");
    let etag = crate::console::send(
        &console,
        "GET",
        "/api/v1/buckets/held/object?key=a.txt",
        Some(&token),
        None,
    )
    .await
    .json()["etag"]
        .as_str()
        .expect("an etag")
        .to_owned();
    let delete_object = format!(
        r#"{{"etag":{},"reason":"probe"}}"#,
        serde_json::Value::from(etag.as_str())
    );
    // Every console route that calls `on_bucket` (`rg 'on_bucket\(' src/console`: 10), the bucket's deletion last.
    let paths: Vec<(&str, &str, Option<&str>)> = vec![
        ("GET", "/api/v1/buckets/held/objects", None),
        ("GET", "/api/v1/buckets/held/object?key=a.txt", None),
        ("GET", "/api/v1/buckets/held/object/content?key=a.txt", None),
        ("GET", "/api/v1/buckets/held/media", None),
        (
            "PUT",
            "/api/v1/buckets/held/media",
            Some(r#"{"rules":[],"reason":"probe"}"#),
        ),
        (
            "PUT",
            "/api/v1/buckets/held/acknowledgement",
            Some(r#"{"level":"majority","reason":"probe"}"#),
        ),
        (
            "PUT",
            "/api/v1/buckets/held/encryption",
            Some(r#"{"algorithm":"AES256","reason":"probe"}"#),
        ),
        (
            "POST",
            "/api/v1/buckets/held/upload-keys",
            Some(r#"{"key":"in.txt","expires_in":600,"reason":"probe"}"#),
        ),
        (
            "DELETE",
            "/api/v1/buckets/held/object?key=a.txt",
            Some(delete_object.as_str()),
        ),
        (
            "DELETE",
            "/api/v1/buckets/held",
            Some(r#"{"reason":"probe"}"#),
        ),
    ];
    let deny_all = DENY_ALL.replace("ann-photos", "held");
    assert_eq!(put_policy(&state, "held", &deny_all).await.status, 204);
    for (method, path, body) in &paths {
        let answer = crate::console::send(&console, method, path, Some(&token), *body).await;
        assert_eq!(answer.status, 403, "{method} {path}");
    }
    println!(
        "console paths: source=10 | enforced=10 | verified={} | exempt=[]",
        paths.len()
    );
    // The control: with the policy lifted none of them is refused.
    assert_eq!(
        call(&state, "DELETE", "/held", vec![("policy", None)], b"")
            .await
            .status,
        204
    );
    for (method, path, body) in &paths {
        let answer = crate::console::send(&console, method, path, Some(&token), *body).await;
        assert_ne!(answer.status, 403, "the control: {method} {path}");
    }
}
