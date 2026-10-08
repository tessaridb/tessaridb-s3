//! A user's access keys through the console, against a real node: issuing is idempotent under a repeated submission
//! (the repeat answers the first key and never a second secret), a user's keys are listed without their secrets, and
//! a revoked key stops signing once its cached resolution is older than the published window, measured on a clock
//! the test moves.

use std::sync::atomic::Ordering;

use serde_json::Value;
use tessari_s3_api::ConsoleState;

use crate::access::Key;
use crate::console::{Seen, send, sign_in};
use crate::console_users::states;
use crate::{IGNORED, call_as};

const TOKEN: &str = "6f1c2d1e-8a7b-4c3d-9e0f-112233445566";

async fn post(console: &ConsoleState, token: &str, path: &str, body: &str) -> Seen {
    send(console, "POST", path, Some(token), Some(body)).await
}

fn ids(listing: &Value) -> Vec<String> {
    listing["keys"]
        .as_array()
        .expect("a list")
        .iter()
        .filter_map(|item| item["access_key_id"].as_str().map(str::to_owned))
        .collect()
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_repeated_issue_answers_the_first_key_and_a_revoked_key_stops_signing() {
    let _why = IGNORED;
    let (state, console, now) = states().await;
    let root = sign_in(&console).await;
    for name in ["kim", "lee"] {
        let body = format!(
            "{{\"name\":\"{name}\",\"space\":\"default\",\"role\":\"member\",\"create_buckets\":true,\
             \"reason\":\"joins\"}}"
        );
        assert_eq!(
            post(&console, &root, "/api/v1/users", &body).await.status,
            201
        );
    }
    let missing = post(
        &console,
        &root,
        "/api/v1/users/kim/keys",
        "{\"reason\":\"first\"}",
    )
    .await;
    assert_eq!(
        (missing.status, missing.code().as_deref()),
        (400, Some("idempotency_key_required"))
    );
    let body = format!("{{\"reason\":\"first\",\"idempotency_key\":\"{TOKEN}\"}}");
    let first = post(&console, &root, "/api/v1/users/kim/keys", &body).await;
    assert_eq!(first.status, 201, "{:?}", first.json());
    let key = Key {
        id: first.json()["access_key_id"]
            .as_str()
            .expect("an id")
            .to_owned(),
        secret: first.json()["secret_access_key"]
            .as_str()
            .expect("a secret")
            .to_owned(),
    };
    // The answer was lost and the form submitted again: the same key, and its secret is not shown a second time.
    let repeat = post(&console, &root, "/api/v1/users/kim/keys", &body).await;
    assert_eq!(repeat.status, 200, "{:?}", repeat.json());
    assert_eq!(
        repeat.json()["access_key_id"].as_str(),
        Some(key.id.as_str())
    );
    assert_eq!(repeat.json()["already_issued"], Value::Bool(true));
    assert!(
        repeat.json().get("secret_access_key").is_none(),
        "{:?}",
        repeat.json()
    );
    let reused = post(&console, &root, "/api/v1/users/lee/keys", &body).await;
    assert_eq!(
        (reused.status, reused.code().as_deref()),
        (409, Some("idempotency_key_reused"))
    );

    let listed = send(&console, "GET", "/api/v1/users/kim/keys", Some(&root), None).await;
    assert_eq!(listed.status, 200, "{:?}", listed.json());
    assert_eq!(
        ids(&listed.json()),
        vec![key.id.clone()],
        "one key, not two"
    );
    assert!(
        !String::from_utf8_lossy(&listed.bytes).contains(&key.secret),
        "never the secret"
    );
    let lee = send(&console, "GET", "/api/v1/users/lee/keys", Some(&root), None).await;
    assert!(ids(&lee.json()).is_empty(), "{:?}", lee.json());

    // The key signs, so its resolution is now cached on this node.
    let signed = call_as(&state, key.pair(), "GET", "/", vec![], vec![], b"").await;
    assert_eq!(signed.status, 200);
    let path = format!("/api/v1/users/kim/keys/{}", key.id);
    let unexplained = send(&console, "DELETE", &path, Some(&root), Some("{}")).await;
    assert_eq!(
        (unexplained.status, unexplained.code().as_deref()),
        (400, Some("reason_required"))
    );
    let foreign = format!("/api/v1/users/lee/keys/{}", key.id);
    let elsewhere = send(
        &console,
        "DELETE",
        &foreign,
        Some(&root),
        Some("{\"reason\":\"x\"}"),
    )
    .await;
    assert_eq!(
        (elsewhere.status, elsewhere.code().as_deref()),
        (404, Some("no_such_key")),
        "another user's key answers as one that does not exist"
    );
    let leaked = Some("{\"reason\":\"leaked\"}");
    let revoked = send(&console, "DELETE", &path, Some(&root), leaked).await;
    assert_eq!(
        revoked.status,
        204,
        "{}",
        String::from_utf8_lossy(&revoked.bytes)
    );
    let again = send(&console, "DELETE", &path, Some(&root), leaked).await;
    assert_eq!(again.status, 404);
    let after = send(&console, "GET", "/api/v1/users/kim/keys", Some(&root), None).await;
    assert!(ids(&after.json()).is_empty(), "{:?}", after.json());
    // Past the published window the cached resolution is not believed and the key is refused.
    now.fetch_add(5, Ordering::AcqRel);
    let refused = call_as(&state, key.pair(), "GET", "/", vec![], vec![], b"").await;
    assert_eq!(
        (refused.status, refused.code.as_deref()),
        (403, Some("InvalidAccessKeyId"))
    );

    let actions = send(&console, "GET", "/api/v1/actions", Some(&root), None)
        .await
        .json();
    let revocations: Vec<Value> = actions["actions"]
        .as_array()
        .expect("actions")
        .iter()
        .filter(|action| {
            action["operation"] == "revoke_key" && action["outcome"] == key.id.as_str()
        })
        .cloned()
        .collect();
    // Like an issue, each attempt is recorded with what it ended in; one of them removed this key.
    assert_eq!(revocations.len(), 1, "{actions:?}");
    assert_eq!(revocations[0]["target"].as_str(), Some("kim"));
    assert_eq!(revocations[0]["reason"].as_str(), Some("leaked"));
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn two_submissions_of_one_form_at_the_same_moment_issue_one_key() {
    let _why = IGNORED;
    let (_, console, _) = states().await;
    let root = sign_in(&console).await;
    let body = "{\"name\":\"kim\",\"space\":\"default\",\"role\":\"member\",\"reason\":\"joins\"}";
    assert_eq!(
        post(&console, &root, "/api/v1/users", body).await.status,
        201
    );
    // Each round is one form submitted twice at once: the commits overlap, so the loser meets the winner's write.
    const ROUNDS: usize = 10;
    let mut issued = Vec::with_capacity(ROUNDS);
    for round in 0..ROUNDS {
        let body = format!(
            "{{\"reason\":\"twice\",\"idempotency_key\":\"race-{round:04}-0123456789ab\"}}"
        );
        let (one, two) = tokio::join!(
            post(&console, &root, "/api/v1/users/kim/keys", &body),
            post(&console, &root, "/api/v1/users/kim/keys", &body),
        );
        let mut statuses = [one.status, two.status];
        statuses.sort_unstable();
        assert_eq!(
            statuses,
            [200, 201],
            "round {round}: {:?} / {:?}",
            one.json(),
            two.json()
        );
        let id = |seen: &Seen| seen.json()["access_key_id"].as_str().map(str::to_owned);
        assert_eq!(id(&one), id(&two), "round {round}: one key, answered twice");
        issued.extend(id(&one));
    }
    let listed = send(&console, "GET", "/api/v1/users/kim/keys", Some(&root), None).await;
    let mut held = ids(&listed.json());
    held.sort();
    issued.sort();
    assert_eq!(held, issued, "one key per form, every one listed");
}
