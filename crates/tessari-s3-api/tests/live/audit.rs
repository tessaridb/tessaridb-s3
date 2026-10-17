//! The per-request audit record: every request a node serves leaves one message in the region's `s3_access_log`
//! topic, named by its request id so an append that is retried is never doubled.

use axum::body::Body;
use axum::http::Request;
use tessari_s3_core::authz::Role;
use tessari_s3_infrastructure::tessaridb::{Answer, MetaPool, Value};
use tessari_s3_storage::audit::AccessRecord;

use crate::access::user_with_key;
use crate::{ACCESS_KEY, IGNORED, SECRET, call, call_as, fresh, send};

fn record(request_id: &str, status: u16, code: &'static str) -> AccessRecord {
    AccessRecord {
        request_id: request_id.to_owned(),
        at_secs: 1_792_000_000,
        method: "GET".to_owned(),
        operation: "GetObject",
        bucket: Some("logged".to_owned()),
        key: Some("a/b.txt".to_owned()),
        principal: "AKIAIOSFODNN7EXAMPLE".to_owned(),
        status,
        code,
        bytes: 5,
        millis: 3,
        client: Some("127.0.0.1".to_owned()),
    }
}

/// Every message in the region's access log, as `(identity, value)`; a string identity comes back quoted, and is
/// answered here without its quotes.
pub(crate) async fn logged(regional: &MetaPool) -> Vec<(String, Value)> {
    let answers = regional
        .run("READ FROM s3_access_log LIMIT 10000;", Vec::new())
        .await
        .expect("read the access log");
    answers
        .into_iter()
        .flat_map(|answer| match answer {
            Answer::Records { records, .. } => records,
            _ => Vec::new(),
        })
        .map(|(id, value)| (id.trim_matches('\'').to_owned(), value))
        .collect()
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_batch_appended_twice_is_logged_once_under_its_request_ids() {
    let _why = IGNORED;
    let (state, regional) = fresh().await;
    let batch = [
        record("AAAA000000000001", 200, "None"),
        record("AAAA000000000002", 404, "NoSuchKey"),
    ];
    let audit = state.storage().audit();
    assert_eq!(audit.append(&batch).await.expect("appended"), 2);
    assert_eq!(
        audit.append(&batch).await.expect("re-appended"),
        2,
        "a batch that landed before counts as written"
    );
    let log = logged(&regional).await;
    assert_eq!(log.len(), 2, "{log:?}");
    let text = format!("{log:?}");
    for expected in [
        "AAAA000000000001",
        "AAAA000000000002",
        "NoSuchKey",
        "GetObject",
        "a/b.txt",
        "127.0.0.1",
    ] {
        assert!(text.contains(expected), "missing {expected} in {text}");
    }
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn every_request_leaves_one_record_and_no_secret() {
    let _why = IGNORED;
    let (state, regional) = fresh().await;
    let bob = user_with_key(state.storage(), "bob", "alpha", Role::Member, false).await;
    let writer = state.audit_writer(64).expect("the access log starts");
    let mut ids = Vec::new();
    for seen in [
        call(&state, "PUT", "/logged", vec![], b"").await,
        call(&state, "PUT", "/logged/a/b.txt", vec![], b"hello").await,
        call(&state, "GET", "/logged/a/b.txt", vec![], b"").await,
        call(&state, "GET", "/logged/none", vec![], b"").await,
        call_as(
            &state,
            bob.pair(),
            "GET",
            "/logged/a/b.txt",
            vec![],
            vec![],
            b"",
        )
        .await,
        send(
            &state,
            Request::get("/logged/a/b.txt")
                .header("host", "localhost")
                .body(Body::empty())
                .expect("a request"),
        )
        .await,
    ] {
        ids.push(
            seen.headers
                .get("x-amz-request-id")
                .and_then(|value| value.to_str().ok())
                .expect("every answer carries its request id")
                .to_owned(),
        );
    }
    // Every record is still queued, and the writer is told to stop before it starts: what it appends is what
    // stopping lands.
    let (stop, stopped) = tokio::sync::watch::channel(false);
    stop.send(true).expect("the writer listens");
    tokio::spawn(writer.run(stopped))
        .await
        .expect("the writer stopped cleanly");
    let log = logged(&regional).await;
    let identities: Vec<&str> = log.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(identities.len(), ids.len(), "{log:?}");
    for id in &ids {
        assert!(
            identities.contains(&id.as_str()),
            "no record for {id}: {log:?}"
        );
    }
    let text = format!("{log:?}");
    for expected in [
        "CreateBucket",
        "PutObject",
        "NoSuchKey",
        "AccessDenied",
        "a/b.txt",
        ACCESS_KEY,
        bob.id.as_str(),
        "anonymous",
    ] {
        assert!(text.contains(expected), "missing {expected} in {text}");
    }
    for secret in [SECRET, bob.secret.as_str(), "Signature", "AWS4-HMAC-SHA256"] {
        assert!(
            !text.contains(secret),
            "{secret} reached the access log: {text}"
        );
    }
}
