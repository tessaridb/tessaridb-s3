//! The console for users, against a real node: a user signs in with its own key, sees only its space's buckets and
//! usage, reaches another user's bucket only as far as its grant, cannot read the action record or the cluster's
//! members, and its session ends once its key is disabled and the cached resolution is older than the window.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use serde_json::Value;
use tessari_s3_api::{ApiState, ConsoleState};
use tessari_s3_core::authz::{Access, Role};

use crate::access::{Key, grant, user_with_key};
use crate::console::{send, sign_in};
use crate::{IGNORED, call, live_config, prepared, scratch_dir, test_node};
use tessari_s3_storage::users::{NewUser, UserCreated};

/// A server state and a console over one storage, both timed by a clock the test moves.
pub(crate) async fn states() -> (ApiState, ConsoleState, Arc<AtomicI64>) {
    // A data directory, so this node has a drive the cluster view would show.
    let config = live_config(test_node(), None, Some(&scratch_dir()));
    let storage = prepared(&config).await;
    let now = Arc::new(AtomicI64::new(ApiState::system_clock()()));
    let read = Arc::clone(&now);
    let clock: tessari_s3_api::Clock = Arc::new(move || read.load(Ordering::Acquire));
    let console = ConsoleState::new(&config, Arc::clone(&clock), storage.clone())
        .expect("a console is configured");
    (ApiState::new(&config, clock, storage), console, now)
}

/// Signs `key` in, answering the session token, or the refusal's status.
pub(crate) async fn sign_in_as(console: &ConsoleState, key: &Key) -> Result<String, u16> {
    let body = format!(
        "{{\"access_key_id\":\"{}\",\"secret_access_key\":\"{}\"}}",
        key.id, key.secret
    );
    let answer = send(console, "POST", "/api/v1/session", None, Some(&body)).await;
    if answer.status != 200 {
        return Err(answer.status);
    }
    Ok(answer.cookie().expect("a session cookie"))
}

fn bucket_names(listing: &Value) -> Vec<String> {
    listing["buckets"]
        .as_array()
        .expect("buckets")
        .iter()
        .filter_map(|bucket| bucket["name"].as_str().map(str::to_owned))
        .collect()
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_user_signs_in_with_its_own_key_and_sees_only_its_space() {
    let _why = IGNORED;
    let (state, console, _) = states().await;
    let ann = user_with_key(state.storage(), "ann", "alpha", Role::Member, true).await;
    assert_eq!(
        call(&state, "PUT", "/rooted", vec![], b"").await.status,
        200
    );

    let wrong = Key {
        id: ann.id.clone(),
        secret: "not-the-secret-0123456789abcdefghijklm".to_owned(),
    };
    assert_eq!(sign_in_as(&console, &wrong).await, Err(401));
    let token = sign_in_as(&console, &ann).await.expect("signed in");
    let created = send(
        &console,
        "POST",
        "/api/v1/buckets",
        Some(&token),
        Some("{\"name\":\"ann-photos\",\"reason\":\"test\"}"),
    )
    .await;
    assert_eq!(created.status, 201, "{:?}", created.json());
    let listed = send(&console, "GET", "/api/v1/buckets", Some(&token), None).await;
    assert_eq!(bucket_names(&listed.json()), vec!["ann-photos"]);
    let root = sign_in(&console).await;
    let all = send(&console, "GET", "/api/v1/buckets", Some(&root), None).await;
    assert_eq!(bucket_names(&all.json()), vec!["ann-photos", "rooted"]);

    assert_eq!(
        call(&state, "PUT", "/rooted/big", vec![], &[7_u8; 100])
            .await
            .status,
        200
    );
    let ann_object = (ann.id.as_str(), ann.secret.as_str());
    let put = crate::call_as(
        &state,
        ann_object,
        "PUT",
        "/ann-photos/a",
        vec![],
        vec![],
        b"abc",
    )
    .await;
    assert_eq!(put.status, 200);
    state
        .storage()
        .usage()
        .refresh("standalone")
        .await
        .expect("measured");
    let usage = send(&console, "GET", "/api/v1/usage", Some(&token), None)
        .await
        .json();
    let measured: Vec<&str> = usage["buckets"]
        .as_array()
        .expect("buckets")
        .iter()
        .filter_map(|bucket| bucket["bucket"].as_str())
        .collect();
    assert_eq!(
        measured,
        vec!["ann-photos"],
        "only the user's own buckets are measured for it"
    );
    assert_eq!(
        (&usage["objects"], &usage["bytes"]),
        (&Value::from(1), &Value::from(3))
    );

    let actions = send(&console, "GET", "/api/v1/actions", Some(&token), None).await;
    assert_eq!(actions.status, 403, "only operators read the action record");
    let status = send(&console, "GET", "/api/v1/status", Some(&token), None).await;
    assert_eq!(status.status, 200);
    assert!(
        status.json().get("members").is_none_or(Value::is_null),
        "no cluster view without the permission"
    );
    assert!(status.json().get("drive").is_none_or(Value::is_null));
    let root_status = send(&console, "GET", "/api/v1/status", Some(&root), None).await;
    assert!(
        root_status.json()["drive"].is_object(),
        "the control: root sees this node's drive"
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_user_reaches_console_data_only_as_far_as_its_grant() {
    let _why = IGNORED;
    let (state, console, now) = states().await;
    let storage = state.storage();
    let ann = user_with_key(storage, "ann", "alpha", Role::Member, true).await;
    let bob = user_with_key(storage, "bob", "alpha", Role::Member, false).await;
    let ann_token = sign_in_as(&console, &ann).await.expect("ann");
    let created = send(
        &console,
        "POST",
        "/api/v1/buckets",
        Some(&ann_token),
        Some("{\"name\":\"ann-photos\",\"reason\":\"test\"}"),
    )
    .await;
    assert_eq!(created.status, 201);
    let bob_token = sign_in_as(&console, &bob).await.expect("bob");

    let objects = "/api/v1/buckets/ann-photos/objects";
    assert_eq!(
        send(&console, "GET", objects, Some(&bob_token), None)
            .await
            .status,
        403
    );
    grant(
        storage,
        "bob",
        "ann-photos",
        Access {
            read: true,
            write: false,
        },
    )
    .await;
    // The grant binds on bob's next resolution: once the cached one is older than the window.
    now.fetch_add(5, Ordering::AcqRel);
    assert_eq!(
        send(&console, "GET", objects, Some(&bob_token), None)
            .await
            .status,
        200
    );
    let removed = send(
        &console,
        "DELETE",
        "/api/v1/buckets/ann-photos",
        Some(&bob_token),
        Some("{\"reason\":\"test\"}"),
    )
    .await;
    assert_eq!(removed.status, 403, "read is not delete");
    let elsewhere = send(
        &console,
        "GET",
        "/api/v1/buckets/nowhere/objects",
        Some(&bob_token),
        None,
    )
    .await;
    assert_eq!(
        elsewhere.status, 403,
        "a bucket the user cannot reach answers alike whether it exists or not"
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_disabled_keys_session_ends_once_its_resolution_is_older_than_the_window() {
    let _why = IGNORED;
    let (state, console, now) = states().await;
    let ann = user_with_key(state.storage(), "ann", "alpha", Role::Member, true).await;
    let token = sign_in_as(&console, &ann).await.expect("signed in");
    assert_eq!(
        send(&console, "GET", "/api/v1/buckets", Some(&token), None)
            .await
            .status,
        200
    );
    assert!(
        state
            .storage()
            .users()
            .set_key_disabled(&ann.id, true)
            .await
            .expect("disabled")
    );
    now.fetch_add(4, Ordering::AcqRel);
    assert_eq!(
        send(&console, "GET", "/api/v1/buckets", Some(&token), None)
            .await
            .status,
        200
    );
    now.fetch_add(1, Ordering::AcqRel);
    assert_eq!(
        send(&console, "GET", "/api/v1/buckets", Some(&token), None)
            .await
            .status,
        401
    );
    assert_eq!(sign_in_as(&console, &ann).await, Err(401));
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn the_cluster_view_is_answered_only_to_who_may_see_it_and_the_session_says_so() {
    let _why = IGNORED;
    let (state, console, _) = states().await;
    let storage = state.storage();
    let ann = user_with_key(storage, "ann", "alpha", Role::Member, true).await;
    let created = storage
        .users()
        .create(&NewUser {
            name: crate::users::user("watcher"),
            space: crate::access::space("alpha"),
            role: Role::Member,
            create_buckets: false,
            operator: false,
            cluster_viewer: true,
        })
        .await
        .expect("created");
    assert!(matches!(created, UserCreated::Created(_)));
    let issued = storage
        .users()
        .issue_key(&crate::users::user("watcher"))
        .await
        .expect("issued")
        .expect("watcher");
    let watcher = Key {
        id: issued.access_key_id,
        secret: issued.secret.expose().to_owned(),
    };
    let ann_token = sign_in_as(&console, &ann).await.expect("ann");
    let viewer_token = sign_in_as(&console, &watcher).await.expect("watcher");
    let root = sign_in(&console).await;

    let refused = send(&console, "GET", "/api/v1/cluster", Some(&ann_token), None).await;
    assert_eq!(refused.status, 403, "a member without the permission");
    for token in [&viewer_token, &root] {
        let seen = send(&console, "GET", "/api/v1/cluster", Some(token), None).await;
        assert_eq!(seen.status, 200);
        let body = seen.json();
        assert!(
            body["metadata"]["addresses"]
                .as_array()
                .is_some_and(|a| !a.is_empty()),
            "{body}"
        );
        assert!(body.get("members").is_some() && body.get("layout").is_some());
        assert!(body["heal_backlog"].is_object(), "{body}");
    }

    let me = |token: &str| {
        let console = console.clone();
        let token = token.to_owned();
        async move {
            send(&console, "GET", "/api/v1/session", Some(&token), None)
                .await
                .json()
        }
    };
    let ann_may = me(&ann_token).await;
    assert_eq!(
        (
            &ann_may["operate"],
            &ann_may["administer"],
            &ann_may["view_cluster"]
        ),
        (
            &Value::Bool(false),
            &Value::Bool(false),
            &Value::Bool(false)
        )
    );
    assert_eq!(ann_may["access_key_id"], Value::from(ann.id.clone()));
    assert_eq!(me(&viewer_token).await["view_cluster"], Value::Bool(true));
    let root_may = me(&root).await;
    assert_eq!(
        (
            &root_may["operate"],
            &root_may["administer"],
            &root_may["view_cluster"]
        ),
        (&Value::Bool(true), &Value::Bool(true), &Value::Bool(true))
    );
}
