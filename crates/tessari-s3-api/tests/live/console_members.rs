//! Cluster membership through the console, against a real node: root approves a node, issues a one-time join token
//! that is shown once and stored nowhere it can be read back, and removes a node for good — but not one the current
//! layout places shards on; a user is refused every route; every act is recorded with its reason.

use serde_json::Value;
use tessari_s3_core::authz::Role;

use crate::IGNORED;
use crate::access::user_with_key;
use crate::console::{send, sign_in};
use crate::console_users::sign_in_as;
use crate::erasure::member;

const MEMBERS: &str = "/api/v1/cluster/members";
const APPROVE: &str = "/api/v1/cluster/members/approve";
const TOKEN: &str = "/api/v1/cluster/members/token";
const REMOVE: &str = "/api/v1/cluster/members/remove";

fn recorded(actions: &Value) -> Vec<(String, String)> {
    actions["actions"]
        .as_array()
        .expect("actions")
        .iter()
        .filter(|action| {
            action["operation"].as_str().is_some_and(|operation| {
                operation.ends_with("_member") || operation == "issue_join_token"
            })
        })
        .map(|action| {
            (
                action["operation"].as_str().unwrap_or_default().to_owned(),
                action["outcome"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn root_approves_issues_a_token_once_and_removes_and_every_act_is_recorded() {
    let _why = IGNORED;
    let member = member().await;
    let root = sign_in(&member.console).await;
    let console = &member.console;
    let print = "a".repeat(64);

    for (body, code) in [
        (r#"{"node":"n7"}"#.to_owned(), "reason_required"),
        (
            r#"{"node":"N 7","reason":"r"}"#.to_owned(),
            "invalid_node_id",
        ),
        (
            r#"{"node":"n7","fingerprint":"AB","reason":"r"}"#.to_owned(),
            "invalid_fingerprint",
        ),
    ] {
        let refused = send(console, "POST", APPROVE, Some(&root), Some(&body)).await;
        assert_eq!(
            (refused.status, refused.code().as_deref()),
            (400, Some(code)),
            "{body}"
        );
    }
    let approve = format!(r#"{{"node":"n7","fingerprint":"{print}","reason":"seventh node"}}"#);
    let approved = send(console, "POST", APPROVE, Some(&root), Some(&approve)).await;
    assert_eq!(approved.status, 200);
    assert_eq!(
        (
            approved.json()["state"].as_str(),
            approved.json()["fingerprint"].as_str()
        ),
        (Some("approved"), Some(print.as_str()))
    );

    let short = send(
        console,
        "POST",
        TOKEN,
        Some(&root),
        Some(r#"{"node":"n8","lifetime_secs":5,"reason":"r"}"#),
    )
    .await;
    assert_eq!(
        (short.status, short.code().as_deref()),
        (400, Some("invalid_lifetime"))
    );
    let issued = send(
        console,
        "POST",
        TOKEN,
        Some(&root),
        Some(r#"{"node":"n8","lifetime_secs":600,"reason":"eighth node"}"#),
    )
    .await;
    assert_eq!(issued.status, 200);
    assert_eq!(issued.header("cache-control"), Some("no-store"));
    let token = issued.json()["token"]
        .as_str()
        .expect("the token")
        .to_owned();
    assert_eq!(token.len(), 64);
    assert!(issued.json()["expires"].is_string());

    let listed = send(console, "GET", MEMBERS, Some(&root), None).await;
    assert_eq!(listed.status, 200);
    let nodes: Vec<(String, String, bool)> = listed.json()["members"]
        .as_array()
        .expect("members")
        .iter()
        .map(|m| {
            (
                m["node"].as_str().unwrap_or_default().to_owned(),
                m["state"].as_str().unwrap_or_default().to_owned(),
                m["awaiting_token"].as_bool().unwrap_or_default(),
            )
        })
        .collect();
    assert_eq!(
        nodes,
        vec![
            ("n7".to_owned(), "approved".to_owned(), false),
            ("n8".to_owned(), "approved".to_owned(), true)
        ]
    );

    let in_layout = send(
        console,
        "POST",
        REMOVE,
        Some(&root),
        Some(r#"{"node":"n1","reason":"retire"}"#),
    )
    .await;
    assert_eq!(
        (in_layout.status, in_layout.code().as_deref()),
        (409, Some("member_in_layout"))
    );
    let unknown = send(
        console,
        "POST",
        REMOVE,
        Some(&root),
        Some(r#"{"node":"n99","reason":"r"}"#),
    )
    .await;
    assert_eq!(
        (unknown.status, unknown.code().as_deref()),
        (404, Some("no_such_member"))
    );
    let removed = send(
        console,
        "POST",
        REMOVE,
        Some(&root),
        Some(r#"{"node":"n7","reason":"decommissioned"}"#),
    )
    .await;
    assert_eq!(removed.status, 204);
    let again = send(
        console,
        "POST",
        REMOVE,
        Some(&root),
        Some(r#"{"node":"n7","reason":"r"}"#),
    )
    .await;
    assert_eq!(
        (again.status, again.code().as_deref()),
        (409, Some("member_removed"))
    );
    let back = send(
        console,
        "POST",
        APPROVE,
        Some(&root),
        Some(r#"{"node":"n7","reason":"back"}"#),
    )
    .await;
    assert_eq!(
        (back.status, back.code().as_deref()),
        (409, Some("member_removed"))
    );

    let actions = send(console, "GET", "/api/v1/actions", Some(&root), None).await;
    let after = send(console, "GET", MEMBERS, Some(&root), None).await;
    for page in [&actions, &after] {
        assert!(
            !String::from_utf8_lossy(&page.bytes).contains(&token),
            "the token was shown again"
        );
    }
    let recorded = recorded(&actions.json());
    for expected in [
        ("approve_member", "approved"),
        ("issue_join_token", "issued"),
        ("remove_member", "member_in_layout"),
        ("remove_member", "removed"),
        ("approve_member", "member_removed"),
    ] {
        assert!(
            recorded.contains(&(expected.0.to_owned(), expected.1.to_owned())),
            "{expected:?} not in {recorded:?}"
        );
    }
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_user_is_refused_every_membership_route() {
    let _why = IGNORED;
    let member = member().await;
    let key = user_with_key(&member.storage, "mem", "mem-space", Role::SpaceAdmin, true).await;
    let user = sign_in_as(&member.console, &key).await.expect("signed in");
    for (method, path, body) in [
        ("GET", MEMBERS, ""),
        ("POST", APPROVE, r#"{"node":"n7","reason":"r"}"#),
        (
            "POST",
            TOKEN,
            r#"{"node":"n8","lifetime_secs":600,"reason":"r"}"#,
        ),
        ("POST", REMOVE, r#"{"node":"n6","reason":"r"}"#),
    ] {
        let body = (!body.is_empty()).then_some(body);
        let answer = send(&member.console, method, path, Some(&user), body).await;
        assert_eq!(answer.status, 403, "{method} {path}");
    }
    let untouched = member.storage.members().list().await.expect("listed");
    assert!(untouched.is_empty(), "a refused route wrote {untouched:?}");
}
