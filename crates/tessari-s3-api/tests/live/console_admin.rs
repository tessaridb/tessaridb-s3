//! Administering users through the console, against a real node: an operator creates spaces and users, a key's
//! secret is shown once and never again, a space admin manages only the plain members of its own space and never
//! itself, a grant's bucket must be in the user's space, and every change is recorded with its reason.

use std::sync::atomic::Ordering;

use serde_json::Value;
use tessari_s3_api::ConsoleState;
use tessari_s3_core::authz::Role;
use tessari_s3_types::BucketName;

use crate::access::{Key, space, user_with_key};
use crate::console::{Seen, send, sign_in};
use crate::console_users::{sign_in_as, states};
use crate::{IGNORED, call_as};

async fn post(console: &ConsoleState, token: &str, path: &str, body: &str) -> Seen {
    send(console, "POST", path, Some(token), Some(body)).await
}

fn names(listing: &Value, field: &str) -> Vec<String> {
    listing[field]
        .as_array()
        .expect("a list")
        .iter()
        .filter_map(|item| item["name"].as_str().map(str::to_owned))
        .collect()
}

/// The key a `POST …/keys` answered.
fn issued(answer: &Seen) -> Key {
    let body = answer.json();
    Key {
        id: body["access_key_id"].as_str().expect("an id").to_owned(),
        secret: body["secret_access_key"]
            .as_str()
            .expect("a secret")
            .to_owned(),
    }
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn an_operator_creates_spaces_and_users_and_a_key_is_shown_once() {
    let _why = IGNORED;
    let (_, console, _) = states().await;
    let root = sign_in(&console).await;

    let refused = post(&console, &root, "/api/v1/spaces", "{\"name\":\"alpha\"}").await;
    assert_eq!(
        (refused.status, refused.code().as_deref()),
        (400, Some("reason_required"))
    );
    let made = post(
        &console,
        &root,
        "/api/v1/spaces",
        "{\"name\":\"alpha\",\"reason\":\"new team\"}",
    )
    .await;
    assert_eq!(made.status, 201, "{:?}", made.json());
    let again = post(
        &console,
        &root,
        "/api/v1/spaces",
        "{\"name\":\"alpha\",\"reason\":\"twice\"}",
    )
    .await;
    assert_eq!(
        (again.status, again.code().as_deref()),
        (409, Some("space_exists"))
    );
    let spaces = send(&console, "GET", "/api/v1/spaces", Some(&root), None).await;
    assert_eq!(names(&spaces.json(), "spaces"), vec!["alpha", "default"]);

    let boss = post(
        &console,
        &root,
        "/api/v1/users",
        "{\"name\":\"boss\",\"space\":\"alpha\",\"role\":\"space_admin\",\"reason\":\"lead\"}",
    )
    .await;
    assert_eq!(boss.status, 201, "{:?}", boss.json());
    let smuggled = post(
        &console,
        &root,
        "/api/v1/users",
        "{\"name\":\"eve\",\"space\":\"alpha\",\"role\":\"member\",\"disabled\":false,\"reason\":\"x\"}",
    )
    .await;
    assert_eq!(smuggled.status, 400, "a field outside the allow-list");
    let key = post(
        &console,
        &root,
        "/api/v1/users/boss/keys",
        "{\"reason\":\"first key\"}",
    )
    .await;
    assert_eq!(key.status, 201, "{:?}", key.json());
    assert_eq!(key.header("cache-control"), Some("no-store"));
    let boss_key = issued(&key);
    let boss_token = sign_in_as(&console, &boss_key)
        .await
        .expect("the key signs in");

    let ann = post(
        &console,
        &boss_token,
        "/api/v1/users",
        "{\"name\":\"ann\",\"space\":\"alpha\",\"role\":\"member\",\"create_buckets\":true,\"reason\":\"joins\"}",
    )
    .await;
    assert_eq!(ann.status, 201, "{:?}", ann.json());
    for (body, why) in [
        (
            "{\"name\":\"peer\",\"space\":\"alpha\",\"role\":\"space_admin\",\"reason\":\"x\"}",
            "an admin is above a space admin's ceiling",
        ),
        (
            "{\"name\":\"ops\",\"space\":\"alpha\",\"role\":\"member\",\"operator\":true,\"reason\":\"x\"}",
            "a store-wide role is above it",
        ),
        (
            "{\"name\":\"bob\",\"space\":\"default\",\"role\":\"member\",\"reason\":\"x\"}",
            "another space",
        ),
    ] {
        assert_eq!(
            post(&console, &boss_token, "/api/v1/users", body)
                .await
                .status,
            403,
            "{why}"
        );
    }
    assert_eq!(
        post(
            &console,
            &boss_token,
            "/api/v1/users/boss/keys",
            "{\"reason\":\"mine\"}"
        )
        .await
        .status,
        403,
        "nobody issues keys to itself"
    );
    assert_eq!(
        post(
            &console,
            &boss_token,
            "/api/v1/spaces",
            "{\"name\":\"gamma\",\"reason\":\"x\"}"
        )
        .await
        .status,
        403
    );
    let other = post(
        &console,
        &root,
        "/api/v1/users",
        "{\"name\":\"bob\",\"space\":\"default\",\"role\":\"member\",\"reason\":\"elsewhere\"}",
    )
    .await;
    assert_eq!(other.status, 201);

    let seen_by_boss = send(&console, "GET", "/api/v1/users", Some(&boss_token), None).await;
    assert_eq!(names(&seen_by_boss.json(), "users"), vec!["ann", "boss"]);
    let seen_by_root = send(&console, "GET", "/api/v1/users", Some(&root), None).await;
    assert_eq!(
        names(&seen_by_root.json(), "users"),
        vec!["ann", "bob", "boss"]
    );

    let actions = send(&console, "GET", "/api/v1/actions", Some(&root), None).await;
    let text = String::from_utf8(actions.json().to_string().into_bytes()).expect("utf-8");
    assert!(
        !text.contains(&boss_key.secret),
        "a secret never enters the record"
    );
    assert!(!seen_by_root.json().to_string().contains(&boss_key.secret));
    let recorded: Vec<(String, String, String)> = actions.json()["actions"]
        .as_array()
        .expect("actions")
        .iter()
        .map(|action| {
            (
                action["operation"].as_str().unwrap_or_default().to_owned(),
                action["target"].as_str().unwrap_or_default().to_owned(),
                action["reason"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    for expected in [
        ("create_space", "alpha", "new team"),
        ("create_user", "boss", "lead"),
        ("issue_key", "boss", "first key"),
        ("create_user", "ann", "joins"),
    ] {
        assert!(
            recorded.contains(&(
                expected.0.to_owned(),
                expected.1.to_owned(),
                expected.2.to_owned()
            )),
            "{expected:?} in {recorded:?}"
        );
    }
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_space_admin_disables_and_grants_only_within_its_ceiling() {
    let _why = IGNORED;
    let (state, console, now) = states().await;
    let storage = state.storage();
    let boss = user_with_key(storage, "boss", "alpha", Role::SpaceAdmin, false).await;
    let ann = user_with_key(storage, "ann", "alpha", Role::Member, true).await;
    user_with_key(storage, "peer", "alpha", Role::SpaceAdmin, false).await;
    user_with_key(storage, "bob", "beta", Role::Member, false).await;
    assert_eq!(
        call_as(
            &state,
            ann.pair(),
            "PUT",
            "/ann-photos",
            vec![],
            vec![],
            b""
        )
        .await
        .status,
        200
    );
    storage
        .buckets()
        .create(
            &BucketName::new("beta-files").expect("a name"),
            "us-east-1",
            &space("beta"),
            None,
        )
        .await
        .expect("created");
    let token = sign_in_as(&console, &boss).await.expect("boss");
    let member = sign_in_as(&console, &ann).await.expect("ann");
    let put = |path: &'static str, body: &'static str| {
        let console = console.clone();
        let token = token.clone();
        async move { send(&console, "PUT", path, Some(&token), Some(body)).await }
    };

    let granted = put(
        "/api/v1/users/ann/grants/ann-photos",
        "{\"read\":true,\"write\":false,\"reason\":\"review\"}",
    )
    .await;
    assert_eq!(granted.status, 204, "{:?}", granted.json());
    for elsewhere in [
        "/api/v1/users/ann/grants/beta-files",
        "/api/v1/users/ann/grants/nowhere",
    ] {
        let refused = put(elsewhere, "{\"read\":true,\"write\":true,\"reason\":\"x\"}").await;
        assert_eq!(
            (refused.status, refused.code().as_deref()),
            (400, Some("bucket_not_in_space")),
            "a bucket of another space answers as a missing one: {elsewhere}"
        );
    }
    for (path, why) in [
        ("/api/v1/users/bob/disabled", "another space"),
        ("/api/v1/users/peer/disabled", "an equal"),
        (
            "/api/v1/users/nobody/disabled",
            "a missing user answers alike",
        ),
        ("/api/v1/users/boss/disabled", "itself"),
    ] {
        assert_eq!(
            put(path, "{\"disabled\":true,\"reason\":\"x\"}")
                .await
                .status,
            403,
            "{why}"
        );
    }
    assert_eq!(
        send(&console, "GET", "/api/v1/users", Some(&member), None)
            .await
            .status,
        403,
        "a member administers nobody"
    );
    let missing_reason = put("/api/v1/users/ann/disabled", "{\"disabled\":true}").await;
    assert_eq!(missing_reason.code().as_deref(), Some("reason_required"));

    let principal = storage
        .users()
        .principal(&crate::users::user("ann"))
        .await
        .expect("answered")
        .expect("ann");
    assert!(
        principal
            .grants
            .get("ann-photos")
            .is_some_and(|a| a.read && !a.write)
    );
    let removed = send(
        &console,
        "DELETE",
        "/api/v1/users/ann/grants/ann-photos",
        Some(&token),
        Some("{\"reason\":\"done\"}"),
    )
    .await;
    assert_eq!(removed.status, 204);
    let twice = send(
        &console,
        "DELETE",
        "/api/v1/users/ann/grants/ann-photos",
        Some(&token),
        Some("{\"reason\":\"done\"}"),
    )
    .await;
    assert_eq!(
        (twice.status, twice.code().as_deref()),
        (404, Some("no_such_grant"))
    );

    assert_eq!(
        put(
            "/api/v1/users/ann/disabled",
            "{\"disabled\":true,\"reason\":\"left\"}"
        )
        .await
        .status,
        204
    );
    now.fetch_add(5, Ordering::AcqRel);
    assert_eq!(
        sign_in_as(&console, &ann).await,
        Err(401),
        "a disabled user's key stops once its resolution is older than the window"
    );
}
