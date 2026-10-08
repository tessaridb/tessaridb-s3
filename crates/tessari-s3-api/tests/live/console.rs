//! The console's data routes against a real node: what an operator changes through the console is exactly what an S3
//! client then sees, every change and every download is in the action record with the operator's key and reason, an
//! object is deleted only under the ETag the operator saw, a download is an attachment that never renders, and a
//! cluster member's status names its members and its heal backlog.

use std::net::SocketAddr;

use axum::body::{Body, to_bytes};
use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, Request};
use serde_json::Value;
use tessari_s3_api::{ConsoleState, console_router};
use tower::ServiceExt;

use crate::erasure_reads::{read_back, shard_file, striped, written};
use crate::{ACCESS_KEY, IGNORED, SECRET, call, call_with, fresh_console};

struct Seen {
    status: u16,
    headers: HeaderMap,
    bytes: Vec<u8>,
}

impl Seen {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.bytes).expect("a JSON body")
    }

    fn code(&self) -> Option<String> {
        self.json()
            .get("code")
            .and_then(Value::as_str)
            .map(str::to_owned)
    }
}

async fn send(
    console: &ConsoleState,
    method: &str,
    path: &str,
    cookie: Option<&str>,
    json: Option<&str>,
) -> Seen {
    let mut request = Request::builder().method(method).uri(path);
    if let Some(token) = cookie {
        request = request.header("cookie", format!("tessaridb_s3_console={token}"));
    }
    let mut request = match json {
        Some(text) => request
            .header("content-type", "application/json")
            .body(Body::from(text.to_owned())),
        None => request.body(Body::empty()),
    }
    .expect("request");
    request
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([10, 0, 0, 9], 40_000))));
    let response = console_router(console.clone())
        .oneshot(request)
        .await
        .expect("infallible");
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), 1 << 24)
        .await
        .expect("body")
        .to_vec();
    Seen {
        status,
        headers,
        bytes,
    }
}

async fn sign_in(console: &ConsoleState) -> String {
    let body = format!("{{\"access_key_id\":\"{ACCESS_KEY}\",\"secret_access_key\":\"{SECRET}\"}}");
    let answer = send(console, "POST", "/api/v1/session", None, Some(&body)).await;
    assert_eq!(answer.status, 200);
    let cookie = answer
        .headers
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .expect("a session cookie");
    cookie
        .trim_start_matches("tessaridb_s3_console=")
        .split(';')
        .next()
        .expect("a value")
        .to_owned()
}

/// The action record, newest first, as (operation, target, reason, outcome).
async fn record(console: &ConsoleState, token: &str) -> Vec<(String, String, Value, String)> {
    let answer = send(console, "GET", "/api/v1/actions", Some(token), None).await;
    assert_eq!(answer.status, 200);
    let field = |action: &Value, name: &str| {
        action
            .get(name)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    answer.json()["actions"]
        .as_array()
        .expect("actions")
        .iter()
        .map(|action| {
            assert_eq!(field(action, "operator"), ACCESS_KEY);
            (
                field(action, "operation"),
                field(action, "target"),
                action.get("reason").cloned().unwrap_or(Value::Null),
                field(action, "outcome"),
            )
        })
        .collect()
}

fn row(
    operation: &str,
    target: &str,
    reason: Option<&str>,
    outcome: &str,
) -> (String, String, Value, String) {
    (
        operation.to_owned(),
        target.to_owned(),
        reason.map_or(Value::Null, |text| Value::String(text.to_owned())),
        outcome.to_owned(),
    )
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn buckets_changed_through_the_console_are_what_s3_sees_and_are_recorded() {
    let _why = IGNORED;
    let (state, console) = fresh_console().await;
    let token = sign_in(&console).await;
    let status = send(&console, "GET", "/api/v1/status", Some(&token), None).await;
    assert_eq!(status.status, 200);
    assert_eq!(
        (&status.json()["members"], &status.json()["heal_backlog"]),
        (&Value::Null, &Value::Null),
        "a single node has neither"
    );
    let create = r#"{"name":"opsbucket","reason":"new team"}"#;
    let created = send(
        &console,
        "POST",
        "/api/v1/buckets",
        Some(&token),
        Some(create),
    )
    .await;
    assert_eq!(created.status, 201);
    assert_eq!(
        call(&state, "HEAD", "/opsbucket", vec![], b"").await.status,
        200
    );
    let again = send(
        &console,
        "POST",
        "/api/v1/buckets",
        Some(&token),
        Some(create),
    )
    .await;
    assert_eq!(
        (again.status, again.code().as_deref()),
        (409, Some("bucket_exists"))
    );
    let listed = send(&console, "GET", "/api/v1/buckets", Some(&token), None).await;
    assert_eq!(listed.json()["buckets"][0]["name"], "opsbucket");
    assert_eq!(
        call(&state, "PUT", "/opsbucket/k", vec![], b"held")
            .await
            .status,
        200
    );
    let reason = Some(r#"{"reason":"team left"}"#);
    let full = send(
        &console,
        "DELETE",
        "/api/v1/buckets/opsbucket",
        Some(&token),
        reason,
    )
    .await;
    assert_eq!(
        (full.status, full.code().as_deref()),
        (409, Some("bucket_not_empty"))
    );
    assert_eq!(
        call(&state, "DELETE", "/opsbucket/k", vec![], b"")
            .await
            .status,
        204
    );
    let unexplained = send(
        &console,
        "DELETE",
        "/api/v1/buckets/opsbucket",
        Some(&token),
        Some("{}"),
    )
    .await;
    assert_eq!(unexplained.code().as_deref(), Some("reason_required"));
    let gone = send(
        &console,
        "DELETE",
        "/api/v1/buckets/opsbucket",
        Some(&token),
        reason,
    )
    .await;
    assert_eq!(gone.status, 204);
    assert_eq!(
        call(&state, "HEAD", "/opsbucket", vec![], b"").await.status,
        404
    );
    assert_eq!(
        record(&console, &token).await,
        vec![
            row("delete_bucket", "opsbucket", Some("team left"), "done"),
            row("delete_bucket", "opsbucket", Some("team left"), "not_empty"),
            row("create_bucket", "opsbucket", Some("new team"), "exists"),
            row("create_bucket", "opsbucket", Some("new team"), "done"),
        ]
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn objects_are_listed_described_downloaded_and_deleted_under_their_etag() {
    let _why = IGNORED;
    let (state, console) = fresh_console().await;
    let token = sign_in(&console).await;
    assert_eq!(call(&state, "PUT", "/files", vec![], b"").await.status, 200);
    let page = b"<script>alert(1)</script>";
    let html = vec![("content-type", "text/html")];
    let put = call_with(&state, "PUT", "/files/a.txt", vec![], html, page).await;
    let etag = put
        .headers
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .expect("etag")
        .to_owned();
    for key in ["b.txt", "dir/c.txt", "dir/d.txt"] {
        let path = format!("/files/{key}");
        assert_eq!(call(&state, "PUT", &path, vec![], b"x").await.status, 200);
    }
    // Page by page, one entry each, with and without the roll-up: the cursor resumes after a key and after a prefix.
    for (delimiter, expected) in [
        ("", vec!["a.txt", "b.txt", "dir/c.txt", "dir/d.txt"]),
        ("&delimiter=/", vec!["a.txt", "b.txt", "dir/"]),
    ] {
        let mut seen = Vec::new();
        let mut cursor = String::new();
        loop {
            let path = format!("/api/v1/buckets/files/objects?limit=1{delimiter}{cursor}");
            let page = send(&console, "GET", &path, Some(&token), None)
                .await
                .json();
            for object in page["objects"].as_array().expect("objects") {
                seen.push(object["key"].as_str().expect("key").to_owned());
            }
            for prefix in page["prefixes"].as_array().expect("prefixes") {
                seen.push(prefix.as_str().expect("prefix").to_owned());
            }
            match page["next"].as_str() {
                Some(next) => cursor = format!("&cursor={}", next.replace('/', "%2F")),
                None => break,
            }
        }
        assert_eq!(seen, expected, "delimiter {delimiter:?}");
    }
    let detail = send(
        &console,
        "GET",
        "/api/v1/buckets/files/object?key=a.txt",
        Some(&token),
        None,
    )
    .await;
    let described = detail.json();
    assert_eq!(
        (described["size"].as_u64(), described["etag"].as_str()),
        (Some(25), Some(etag.as_str()))
    );
    assert_eq!(described["headers"]["content-type"], "text/html");
    let path = "/api/v1/buckets/files/object/content?key=a.txt&reason=support%20ticket%207";
    let download = send(&console, "GET", path, Some(&token), None).await;
    assert_eq!(
        (download.status, download.bytes.as_slice()),
        (200, page.as_slice())
    );
    let header = |name: &str| {
        download
            .headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned()
    };
    assert_eq!(
        header("content-type"),
        "application/octet-stream",
        "never rendered"
    );
    assert_eq!(
        header("content-disposition"),
        "attachment; filename*=UTF-8''a.txt"
    );
    assert_eq!(header("x-content-type-options"), "nosniff");
    let stale = r#"{"etag":"\"0123456789abcdef0123456789abcdef\"","reason":"cleanup"}"#;
    let target = "/api/v1/buckets/files/object?key=a.txt";
    let refused = send(&console, "DELETE", target, Some(&token), Some(stale)).await;
    assert_eq!(
        (refused.status, refused.code().as_deref()),
        (412, Some("precondition_failed"))
    );
    assert_eq!(
        call(&state, "GET", "/files/a.txt", vec![], b"")
            .await
            .status,
        200,
        "kept"
    );
    let current = format!(
        r#"{{"etag":{},"reason":"cleanup"}}"#,
        Value::String(etag.clone())
    );
    let deleted = send(&console, "DELETE", target, Some(&token), Some(&current)).await;
    assert_eq!(deleted.status, 204);
    assert_eq!(
        call(&state, "GET", "/files/a.txt", vec![], b"")
            .await
            .status,
        404,
        "gone"
    );
    let missing = send(&console, "GET", target, Some(&token), None).await;
    assert_eq!(
        (missing.status, missing.code().as_deref()),
        (404, Some("no_such_key"))
    );
    let nowhere = send(
        &console,
        "GET",
        "/api/v1/buckets/nowhere/objects",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(
        (nowhere.status, nowhere.code().as_deref()),
        (404, Some("no_such_bucket"))
    );
    assert_eq!(
        record(&console, &token).await,
        vec![
            row("delete_object", "files/a.txt", Some("cleanup"), "done"),
            row(
                "delete_object",
                "files/a.txt",
                Some("cleanup"),
                "precondition_failed"
            ),
            row(
                "download_object",
                "files/a.txt",
                Some("support ticket 7"),
                "sent"
            ),
        ]
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_members_status_names_its_members_and_its_heal_backlog() {
    let _why = IGNORED;
    let member = written().await;
    let token = sign_in(&member.console).await;
    let status = send(&member.console, "GET", "/api/v1/status", Some(&token), None)
        .await
        .json();
    let members: Vec<&str> = status["members"]
        .as_array()
        .expect("members")
        .iter()
        .map(|m| m["node"].as_str().expect("node"))
        .collect();
    assert_eq!(members, vec!["n1", "n2", "n3", "n4", "n5", "n6"]);
    assert_eq!(
        (status["node"].as_str(), status["erasure"].as_str()),
        (Some("n1"), Some("4+2"))
    );
    assert_eq!(
        status["heal_backlog"],
        serde_json::json!({"listed": 0, "more": false})
    );
    std::fs::remove_file(shard_file(&member, 1)).expect("shard removed");
    assert_eq!(read_back(&member).await, striped(), "the read lists it");
    let status = send(&member.console, "GET", "/api/v1/status", Some(&token), None)
        .await
        .json();
    assert_eq!(
        status["heal_backlog"],
        serde_json::json!({"listed": 1, "more": false})
    );
}
