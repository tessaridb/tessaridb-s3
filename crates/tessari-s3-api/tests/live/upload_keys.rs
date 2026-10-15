//! The one-key upload credential, against a real node: a user who may write a bucket issues it through the console;
//! it uploads exactly that key — one PUT or every step of a multipart upload — and nothing else; it stops at its
//! expiry (on a clock the test moves) and once its issuer may no longer write there; it signs nothing in the console.

use std::sync::atomic::Ordering;

use serde_json::{Value, json};
use tessari_s3_api::{ApiState, ConsoleState};
use tessari_s3_core::authz::{Access, Role, UserName};
use tessari_s3_types::BucketName;

use crate::access::{Key, grant, user_with_key};
use crate::console::{Seen, send, sign_in};
use crate::console_users::{sign_in_as, states};
use crate::multipart::{etag, text};
use crate::{IGNORED, call, call_as};

/// The published revocation window, in seconds: a resolution younger than this is believed.
const WINDOW_SECS: i64 = 5;

/// Asks the console, signed in as `token`, for an upload key on `bucket`.
async fn ask(console: &ConsoleState, token: &str, bucket: &str, body: &Value) -> Seen {
    send(
        console,
        "POST",
        &format!("/api/v1/buckets/{bucket}/upload-keys"),
        Some(token),
        Some(&body.to_string()),
    )
    .await
}

/// Issues an upload key for `bucket/key` lasting `seconds`, as `token`; panics unless it was issued.
async fn issue(console: &ConsoleState, token: &str, bucket: &str, key: &str, seconds: i64) -> Key {
    let body = json!({"key": key, "expires_in": seconds, "reason": "a camera upload"});
    let answer = ask(console, token, bucket, &body).await;
    assert_eq!(answer.status, 201, "{:?}", answer.json());
    assert_eq!(answer.header("cache-control"), Some("no-store"));
    let issued = answer.json();
    Key {
        id: issued["access_key_id"].as_str().expect("an id").to_owned(),
        secret: issued["secret_access_key"]
            .as_str()
            .expect("a secret")
            .to_owned(),
    }
}

/// A request signed with `key`, carrying `query` and `body`.
async fn with(
    state: &ApiState,
    key: &Key,
    method: &str,
    path: &str,
    query: Vec<(&str, Option<&str>)>,
    body: &[u8],
) -> crate::Seen {
    call_as(state, key.pair(), method, path, query, vec![], body).await
}

/// The status and S3 error code `key` gets for a PUT of `path`.
async fn put(state: &ApiState, key: &Key, path: &str) -> (u16, Option<String>) {
    let seen = with(state, key, "PUT", path, vec![], b"photo").await;
    (seen.status, seen.code)
}

fn denied() -> (u16, Option<String>) {
    (403, Some("AccessDenied".to_owned()))
}

fn unknown() -> (u16, Option<String>) {
    (403, Some("InvalidAccessKeyId".to_owned()))
}

fn user(name: &str) -> UserName {
    UserName::new(name).expect("a user name")
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn an_upload_key_uploads_its_one_key_and_does_nothing_else() {
    let _why = IGNORED;
    let (state, console, _) = states().await;
    let ann = user_with_key(state.storage(), "ann", "alpha", Role::Member, true).await;
    for bucket in ["/media", "/other"] {
        let created = with(&state, &ann, "PUT", bucket, vec![], b"").await;
        assert_eq!(created.status, 200, "{bucket}");
    }
    let token = sign_in_as(&console, &ann).await.expect("ann signs in");
    let up = issue(&console, &token, "media", "in/a.jpg", 3600).await;

    assert_eq!(put(&state, &up, "/media/in/a.jpg").await, (200, None));
    let stored = call(&state, "GET", "/media/in/a.jpg", vec![], b"").await;
    assert_eq!(
        (stored.status, stored.bytes.as_slice()),
        (200, &b"photo"[..])
    );

    for refused in ["/media/in/b.jpg", "/media/in/a.jpg.bak", "/other/in/a.jpg"] {
        assert_eq!(put(&state, &up, refused).await, denied(), "PUT {refused}");
    }
    for (method, path) in [
        ("GET", "/media/in/a.jpg"),
        ("HEAD", "/media/in/a.jpg"),
        ("DELETE", "/media/in/a.jpg"),
        ("GET", "/media"),
        ("GET", "/"),
        ("PUT", "/fresh"),
    ] {
        let seen = with(&state, &up, method, path, vec![], b"").await;
        assert_eq!(seen.status, 403, "{method} {path}");
    }
    let copied = call_as(
        &state,
        up.pair(),
        "PUT",
        "/media/in/a.jpg",
        vec![],
        vec![("x-amz-copy-source", "/other/x")],
        b"",
    )
    .await;
    assert_eq!((copied.status, copied.code), denied(), "CopyObject");

    // Every step of a multipart upload of the key.
    let path = "/media/in/a.jpg";
    let created = with(&state, &up, "POST", path, vec![("uploads", None)], b"").await;
    assert_eq!(created.status, 200, "{}", created.body);
    let upload = text(&created.body, "UploadId").expect("an upload id");
    let id = Some(upload.as_str());
    let part_query = vec![("partNumber", Some("1")), ("uploadId", id)];
    let part = with(&state, &up, "PUT", path, part_query, b"one part").await;
    assert_eq!(part.status, 200, "{}", part.body);
    let listed = with(&state, &up, "GET", path, vec![("uploadId", id)], b"").await;
    assert_eq!(listed.status, 200, "{}", listed.body);
    let document = format!(
        "<CompleteMultipartUpload><Part><PartNumber>1</PartNumber><ETag>{}</ETag></Part>\
         </CompleteMultipartUpload>",
        etag(&part)
    );
    let completed = with(
        &state,
        &up,
        "POST",
        path,
        vec![("uploadId", id)],
        document.as_bytes(),
    )
    .await;
    assert_eq!(completed.status, 200, "{}", completed.body);
    let stored = call(&state, "GET", path, vec![], b"").await;
    assert_eq!(stored.bytes.as_slice(), b"one part");
    // Abort is a step of uploading too; an upload of another key is not reachable through this one.
    let again = with(&state, &up, "POST", path, vec![("uploads", None)], b"").await;
    let again = text(&again.body, "UploadId").expect("an upload id");
    let aborted = with(
        &state,
        &up,
        "DELETE",
        path,
        vec![("uploadId", Some(&again))],
        b"",
    )
    .await;
    assert_eq!(aborted.status, 204);
    let elsewhere = with(
        &state,
        &up,
        "POST",
        "/media/in/b.jpg",
        vec![("uploads", None)],
        b"",
    )
    .await;
    assert_eq!((elsewhere.status, elsewhere.code), denied());

    // It signs nothing in the console: refused as an unknown key is.
    assert_eq!(sign_in_as(&console, &up).await, Err(401));

    let root = sign_in(&console).await;
    let actions = send(&console, "GET", "/api/v1/actions", Some(&root), None).await;
    let issued: Vec<(String, String, String)> = actions.json()["actions"]
        .as_array()
        .expect("actions")
        .iter()
        .filter(|action| action["operation"] == "issue_upload_key")
        .map(|action| {
            let field = |name: &str| action[name].as_str().unwrap_or_default().to_owned();
            (field("target"), field("reason"), field("outcome"))
        })
        .collect();
    let expected = ("media/in/a.jpg", "a camera upload", up.id.as_str());
    assert_eq!(
        issued
            .iter()
            .map(|(t, r, o)| (t.as_str(), r.as_str(), o.as_str()))
            .collect::<Vec<_>>(),
        vec![expected]
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn an_upload_key_stops_at_its_expiry_and_with_its_issuers_authority() {
    let _why = IGNORED;
    let (state, console, now) = states().await;
    let ann = user_with_key(state.storage(), "ann", "alpha", Role::Member, true).await;
    assert_eq!(
        with(&state, &ann, "PUT", "/media", vec![], b"")
            .await
            .status,
        200
    );
    let token = sign_in_as(&console, &ann).await.expect("ann signs in");

    // Expiry: works one second before, refused at the instant — judged on every request, not when cached.
    let brief = issue(&console, &token, "media", "a", 60).await;
    assert_eq!(put(&state, &brief, "/media/a").await, (200, None));
    now.fetch_add(59, Ordering::AcqRel);
    assert_eq!(put(&state, &brief, "/media/a").await, (200, None));
    now.fetch_add(1, Ordering::AcqRel);
    assert_eq!(put(&state, &brief, "/media/a").await, unknown());

    // The issuer loses the grant: the key loses it with them, within the window.
    let bob = user_with_key(state.storage(), "bob", "alpha", Role::Member, false).await;
    let write = Access {
        read: false,
        write: true,
    };
    grant(state.storage(), "bob", "media", write).await;
    let bob_token = sign_in_as(&console, &bob).await.expect("bob signs in");
    let delegated = issue(&console, &bob_token, "media", "b", 3600).await;
    assert_eq!(put(&state, &delegated, "/media/b").await, (200, None));
    let media = BucketName::new("media").expect("a bucket name");
    let users = state.storage().users();
    assert!(
        users
            .remove_grant(&user("bob"), &media)
            .await
            .expect("removed")
    );
    now.fetch_add(WINDOW_SECS, Ordering::AcqRel);
    assert_eq!(put(&state, &delegated, "/media/b").await, denied());

    // The issuer is disabled: the key resolves to nothing, within the window.
    let lasting = issue(&console, &token, "media", "c", 3600).await;
    assert_eq!(put(&state, &lasting, "/media/c").await, (200, None));
    assert!(
        users
            .set_disabled(&user("ann"), true)
            .await
            .expect("disabled")
    );
    now.fetch_add(WINDOW_SECS, Ordering::AcqRel);
    assert_eq!(put(&state, &lasting, "/media/c").await, unknown());
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn only_a_user_who_may_write_the_bucket_issues_one_within_bounds() {
    let _why = IGNORED;
    let (state, console, _) = states().await;
    let ann = user_with_key(state.storage(), "ann", "alpha", Role::Member, true).await;
    assert_eq!(
        with(&state, &ann, "PUT", "/media", vec![], b"")
            .await
            .status,
        200
    );
    assert_eq!(
        call(&state, "PUT", "/rooted", vec![], b"").await.status,
        200
    );
    let reader = user_with_key(state.storage(), "rita", "alpha", Role::Member, false).await;
    let read = Access {
        read: true,
        write: false,
    };
    grant(state.storage(), "rita", "media", read).await;
    let valid = json!({"key": "a", "expires_in": 3600, "reason": "upload"});

    let root = sign_in(&console).await;
    let by_root = ask(&console, &root, "media", &valid).await;
    assert_eq!(
        (by_root.status, by_root.code().as_deref()),
        (403, Some("issued_by_a_user"))
    );
    let rita = sign_in_as(&console, &reader).await.expect("rita signs in");
    for bucket in ["media", "rooted", "nowhere"] {
        let refused = ask(&console, &rita, bucket, &valid).await;
        assert_eq!(
            (refused.status, refused.code().as_deref()),
            (403, Some("forbidden")),
            "{bucket}"
        );
    }

    let token = sign_in_as(&console, &ann).await.expect("ann signs in");
    // The page offers the action to a user and not to root; the server decides either way.
    for (who, offered) in [(&token, true), (&root, false)] {
        let may = send(&console, "GET", "/api/v1/session", Some(who), None).await;
        assert_eq!(may.json()["issue_upload_keys"], Value::Bool(offered));
    }
    let too_long = "k".repeat(1025);
    for (body, code) in [
        (
            json!({"key": "a", "expires_in": 59, "reason": "x"}),
            "invalid_expiry",
        ),
        (
            json!({"key": "a", "expires_in": 604_801, "reason": "x"}),
            "invalid_expiry",
        ),
        (
            json!({"key": "", "expires_in": 3600, "reason": "x"}),
            "invalid_key",
        ),
        (
            json!({"key": too_long, "expires_in": 3600, "reason": "x"}),
            "invalid_key",
        ),
        (json!({"key": "a", "expires_in": 3600}), "reason_required"),
        (
            json!({"key": "a", "expires_in": 3600, "reason": "x", "bucket": "other"}),
            "bad_request",
        ),
    ] {
        let refused = ask(&console, &token, "media", &body).await;
        assert_eq!(
            (refused.status, refused.code().as_deref()),
            (400, Some(code)),
            "{body}"
        );
    }
    // The widest bound is accepted, and the answer says when the key stops.
    let widest = json!({"key": "a", "expires_in": 604_800, "reason": "a week"});
    let answer = ask(&console, &token, "media", &widest).await;
    assert_eq!(answer.status, 201);
    let expires = answer.json()["expires"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(expires.ends_with('Z') && expires.len() == 24, "{expires}");
}
