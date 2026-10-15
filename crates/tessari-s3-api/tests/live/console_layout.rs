//! Changing a cluster's layout through the console, against a real node: root stages a layout, sees it, and applies it
//! as the next version; a layout naming an unregistered node or a stale apply is refused with its own code; nothing
//! is applied unseen; a user is refused; every act is recorded with its reason.

use serde_json::Value;
use tessari_s3_core::authz::Role;
use tessari_s3_types::NodeId;

use crate::IGNORED;
use crate::access::user_with_key;
use crate::console::{send, sign_in};
use crate::console_users::sign_in_as;
use crate::erasure::member;

const STAGE: &str = r#"{"sets":[["n1","n2","n3","n4","n5","n6"],["n7","n8","n9","n10","n11","n12"]],"reason":"grow to two sets"}"#;

fn versions(cluster: &Value) -> (u64, usize) {
    (
        cluster["layout"]["version"].as_u64().expect("a version"),
        cluster["layout"]["sets"].as_array().expect("sets").len(),
    )
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn root_stages_and_applies_a_layout_and_every_act_is_recorded() {
    let _why = IGNORED;
    let member = member().await;
    let root = sign_in(&member.console).await;
    // The first layout is made when it is first asked for: n1..n6, before the new nodes register.
    let first = send(&member.console, "GET", "/api/v1/cluster", Some(&root), None).await;
    assert_eq!(versions(&first.json()), (1, 1));
    for n in 7..=12 {
        member
            .storage
            .cluster()
            .register(&NodeId::new(&format!("n{n}")).expect("an id"), "x:1", None)
            .await
            .expect("registered");
    }
    let path = "/api/v1/cluster/layout/staged";
    let console = &member.console;

    let none = send(console, "GET", path, Some(&root), None).await;
    assert_eq!(
        (none.status, none.code().as_deref()),
        (404, Some("nothing_staged"))
    );
    let unknown = send(
        console,
        "PUT",
        path,
        Some(&root),
        Some(r#"{"sets":[["n1","n2","n3","n4","n5","n99"]],"reason":"r"}"#),
    )
    .await;
    assert_eq!(
        (unknown.status, unknown.code().as_deref()),
        (400, Some("unregistered_node"))
    );
    let crowded = send(
        console,
        "PUT",
        path,
        Some(&root),
        Some(r#"{"sets":[["n1","n2","n3"]],"reason":"r"}"#),
    )
    .await;
    assert_eq!(
        (crowded.status, crowded.code().as_deref()),
        (400, Some("invalid_layout"))
    );
    let unexplained = send(
        console,
        "PUT",
        path,
        Some(&root),
        Some(r#"{"sets":[["n1","n2","n3","n4","n5","n6"]]}"#),
    )
    .await;
    assert_eq!(unexplained.code().as_deref(), Some("reason_required"));

    let staged = send(console, "PUT", path, Some(&root), Some(STAGE)).await;
    assert_eq!(staged.status, 200);
    let staged = staged.json();
    assert_eq!(
        (
            staged["base"].as_u64(),
            staged["layout"]["version"].as_u64()
        ),
        (Some(1), Some(2))
    );
    let id = staged["id"].as_str().expect("an id").to_owned();
    assert_eq!(
        send(console, "GET", path, Some(&root), None).await.json(),
        staged
    );

    let stale = send(
        console,
        "POST",
        "/api/v1/cluster/layout/apply",
        Some(&root),
        Some(r#"{"id":"not-this-one","reason":"r"}"#),
    )
    .await;
    assert_eq!(
        (stale.status, stale.code().as_deref()),
        (409, Some("staged_changed"))
    );
    let before = send(console, "GET", "/api/v1/cluster", Some(&root), None).await;
    assert_eq!(versions(&before.json()), (1, 1), "nothing applied unseen");

    let apply = format!(r#"{{"id":"{id}","reason":"grow to two sets"}}"#);
    let applied = send(
        console,
        "POST",
        "/api/v1/cluster/layout/apply",
        Some(&root),
        Some(&apply),
    )
    .await;
    assert_eq!(applied.status, 200);
    let after = send(console, "GET", "/api/v1/cluster", Some(&root), None).await;
    assert_eq!(versions(&after.json()), (2, 2));
    let again = send(
        console,
        "POST",
        "/api/v1/cluster/layout/apply",
        Some(&root),
        Some(&apply),
    )
    .await;
    assert_eq!(
        (again.status, again.code().as_deref()),
        (404, Some("nothing_staged")),
        "a repeated apply does not apply twice"
    );

    let restaged = send(console, "PUT", path, Some(&root), Some(STAGE))
        .await
        .json();
    assert_eq!(
        restaged["base"].as_u64(),
        Some(2),
        "staged over the new version"
    );
    let discarded = send(
        console,
        "DELETE",
        path,
        Some(&root),
        Some(r#"{"reason":"changed my mind"}"#),
    )
    .await;
    assert_eq!(discarded.status, 204);
    let gone = send(console, "GET", path, Some(&root), None).await;
    assert_eq!(gone.code().as_deref(), Some("nothing_staged"));

    let actions = send(console, "GET", "/api/v1/actions", Some(&root), None)
        .await
        .json();
    let recorded: Vec<(String, String)> = actions["actions"]
        .as_array()
        .expect("actions")
        .iter()
        .filter(|action| {
            action["operation"]
                .as_str()
                .is_some_and(|operation| operation.ends_with("_layout"))
        })
        .map(|action| {
            (
                action["operation"].as_str().unwrap_or_default().to_owned(),
                action["outcome"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    for expected in [
        ("stage_layout", "staged"),
        ("stage_layout", "unregistered_node"),
        ("apply_layout", "staged_changed"),
        ("apply_layout", "applied"),
        ("discard_layout", "done"),
    ] {
        assert!(
            recorded.contains(&(expected.0.to_owned(), expected.1.to_owned())),
            "{expected:?} not in {recorded:?}"
        );
    }
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_user_may_read_nothing_staged_but_change_nothing() {
    let _why = IGNORED;
    let member = member().await;
    let key = user_with_key(&member.storage, "lay", "lay-space", Role::SpaceAdmin, true).await;
    let user = sign_in_as(&member.console, &key).await.expect("signed in");
    for (method, path, body) in [
        ("PUT", "/api/v1/cluster/layout/staged", STAGE),
        (
            "DELETE",
            "/api/v1/cluster/layout/staged",
            r#"{"reason":"r"}"#,
        ),
        (
            "POST",
            "/api/v1/cluster/layout/apply",
            r#"{"id":"x","reason":"r"}"#,
        ),
        ("GET", "/api/v1/cluster/layout/staged", ""),
    ] {
        let body = (!body.is_empty()).then_some(body);
        let answer = send(&member.console, method, path, Some(&user), body).await;
        assert_eq!(answer.status, 403, "{method} {path}");
    }
}
