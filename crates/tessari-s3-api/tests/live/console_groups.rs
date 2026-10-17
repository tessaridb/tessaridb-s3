//! A space's groups through the console, against a real node: its admin creates a group, adds a member of the space
//! and grants a bucket of the space, and the member then reads that bucket over S3; a member of the space and the
//! admin of another space are refused every group route; a user or a bucket of another space answers as missing.

use tessari_s3_core::authz::Role;

use crate::access::user_with_key;
use crate::console::send;
use crate::console_users::{sign_in_as, states};
use crate::{IGNORED, call_as};

const REASON: &str = "{\"reason\":\"test\"}";

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_space_admin_manages_its_groups_and_a_member_reads_through_one() {
    let _why = IGNORED;
    let (state, console, _) = states().await;
    let storage = state.storage();
    let admin = user_with_key(storage, "adm", "alpha", Role::SpaceAdmin, true).await;
    let ann = user_with_key(storage, "ann", "alpha", Role::Member, false).await;
    let _carl = user_with_key(storage, "carl", "beta", Role::Member, true).await;
    let other = user_with_key(storage, "bea", "beta", Role::SpaceAdmin, true).await;
    for (key, path) in [
        (&admin, "/shared"),
        (&admin, "/shared/a.txt"),
        (&other, "/theirs"),
    ] {
        let made = call_as(&state, key.pair(), "PUT", path, vec![], vec![], b"hi").await;
        assert_eq!(made.status, 200, "{path}: {}", made.body);
    }
    let token = sign_in_as(&console, &admin).await.expect("signed in");
    let at = |path: &str| format!("/api/v1/spaces/alpha/groups{path}");
    let created = send(
        &console,
        "POST",
        &at(""),
        Some(&token),
        Some("{\"name\":\"readers\",\"reason\":\"test\"}"),
    )
    .await;
    assert_eq!(created.status, 201, "{:?}", created.json());
    assert_eq!(
        created.json()["name"],
        serde_json::Value::from("readers"),
        "a create answers what it made, as every console create does"
    );
    let again = send(
        &console,
        "POST",
        &at(""),
        Some(&token),
        Some("{\"name\":\"readers\",\"reason\":\"test\"}"),
    )
    .await;
    assert_eq!(
        (again.status, again.code().as_deref()),
        (409, Some("group_exists"))
    );
    let member = send(
        &console,
        "PUT",
        &at("/readers/members/ann"),
        Some(&token),
        Some(REASON),
    )
    .await;
    assert_eq!(member.status, 204, "{:?}", member.json());
    let granted = send(
        &console,
        "PUT",
        &at("/readers/grants/shared"),
        Some(&token),
        Some("{\"read\":true,\"write\":false,\"reason\":\"test\"}"),
    )
    .await;
    assert_eq!(granted.status, 204, "{:?}", granted.json());
    let elsewhere_user = send(
        &console,
        "PUT",
        &at("/readers/members/carl"),
        Some(&token),
        Some(REASON),
    )
    .await;
    assert_eq!(
        (elsewhere_user.status, elsewhere_user.code().as_deref()),
        (404, Some("no_such_user"))
    );
    let elsewhere_bucket = send(
        &console,
        "PUT",
        &at("/readers/grants/theirs"),
        Some(&token),
        Some("{\"read\":true,\"write\":true,\"reason\":\"test\"}"),
    )
    .await;
    assert_eq!(
        (elsewhere_bucket.status, elsewhere_bucket.code().as_deref()),
        (400, Some("bucket_not_in_space"))
    );
    let detail = send(&console, "GET", &at("/readers"), Some(&token), None)
        .await
        .json();
    assert_eq!(detail["members"], serde_json::json!(["ann"]));
    assert_eq!(
        detail["grants"],
        serde_json::json!([{"bucket": "shared", "read": true, "write": false}])
    );
    let listed = send(&console, "GET", &at(""), Some(&token), None)
        .await
        .json();
    assert_eq!(listed["groups"][0]["name"], "readers");

    let read = call_as(
        &state,
        ann.pair(),
        "GET",
        "/shared/a.txt",
        vec![],
        vec![],
        b"",
    )
    .await;
    assert_eq!((read.status, read.body.as_str()), (200, "hi"));

    // Neither a member of the space nor the admin of another space reaches its groups.
    for key in [&ann, &other] {
        let session = sign_in_as(&console, key).await.expect("signed in");
        for (method, path, body) in [
            ("GET", at(""), None),
            ("GET", at("/readers"), None),
            ("DELETE", at("/readers"), Some(REASON)),
            ("PUT", at("/readers/members/ann"), Some(REASON)),
            ("DELETE", at("/readers/grants/shared"), Some(REASON)),
        ] {
            let refused = send(&console, method, &path, Some(&session), body).await;
            assert_eq!(refused.status, 403, "{method} {path}");
        }
    }

    let removed = send(
        &console,
        "DELETE",
        &at("/readers"),
        Some(&token),
        Some(REASON),
    )
    .await;
    assert_eq!(removed.status, 204);
    let gone = send(&console, "GET", &at("/readers"), Some(&token), None).await;
    assert_eq!(
        (gone.status, gone.code().as_deref()),
        (404, Some("no_such_group"))
    );
}
