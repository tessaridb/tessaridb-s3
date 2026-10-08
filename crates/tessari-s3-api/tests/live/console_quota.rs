//! Setting a bucket's quota through the console, against a real node: only an operator may, with a reason that is
//! recorded; the bucket list shows the limits; a bucket that is not there, a limit the store cannot hold, an unknown
//! field and a missing reason are refused, and nothing is changed by a refusal.

use serde_json::{Value, json};
use tessari_s3_core::authz::Role;

use crate::access::user_with_key;
use crate::console::{send, sign_in};
use crate::console_users::{sign_in_as, states};
use crate::{IGNORED, call};

const QUOTA: &str = "/api/v1/buckets/media/quota";

fn limits_of(listing: &Value, bucket: &str) -> (Value, Value) {
    let row = listing["buckets"]
        .as_array()
        .expect("buckets")
        .iter()
        .find(|row| row["name"] == bucket)
        .expect("the bucket is listed")
        .clone();
    (row["max_bytes"].clone(), row["max_objects"].clone())
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn only_an_operator_sets_a_quota_and_the_reason_is_recorded() {
    let _why = IGNORED;
    let (state, console, _) = states().await;
    assert_eq!(call(&state, "PUT", "/media", vec![], b"").await.status, 200);
    let root = sign_in(&console).await;
    let listed = send(&console, "GET", "/api/v1/buckets", Some(&root), None).await;
    assert_eq!(
        limits_of(&listed.json(), "media"),
        (Value::Null, Value::Null),
        "a new bucket has no limit"
    );

    let set =
        json!({"max_bytes": 1_000_000, "max_objects": null, "reason": "tenant plan"}).to_string();
    for (role, name) in [(Role::Member, "ann"), (Role::SpaceAdmin, "boss")] {
        let key = user_with_key(state.storage(), name, "alpha", role, true).await;
        let token = sign_in_as(&console, &key).await.expect("signed in");
        let refused = send(&console, "PUT", QUOTA, Some(&token), Some(&set)).await;
        assert_eq!(refused.status, 403, "{name}");
    }
    for (body, code) in [
        (
            json!({"max_bytes": 5, "max_objects": 1}).to_string(),
            "reason_required",
        ),
        (
            json!({"max_bytes": 5, "max_objects": 1, "reason": "x", "space": "beta"}).to_string(),
            "bad_request",
        ),
        (
            json!({"max_bytes": u64::MAX, "max_objects": null, "reason": "x"}).to_string(),
            "invalid_quota",
        ),
    ] {
        let refused = send(&console, "PUT", QUOTA, Some(&root), Some(&body)).await;
        assert_eq!(
            (refused.status, refused.code().as_deref()),
            (400, Some(code)),
            "{body}"
        );
    }
    let listed = send(&console, "GET", "/api/v1/buckets", Some(&root), None).await;
    assert_eq!(
        limits_of(&listed.json(), "media"),
        (Value::Null, Value::Null),
        "nothing was changed by a refusal"
    );

    let done = send(&console, "PUT", QUOTA, Some(&root), Some(&set)).await;
    assert_eq!(done.status, 204, "{:?}", done.json());
    let again = send(&console, "PUT", QUOTA, Some(&root), Some(&set)).await;
    assert_eq!(
        again.status, 204,
        "setting the same limits again changes nothing"
    );
    let listed = send(&console, "GET", "/api/v1/buckets", Some(&root), None).await;
    assert_eq!(
        limits_of(&listed.json(), "media"),
        (json!(1_000_000), Value::Null)
    );
    let missing = send(
        &console,
        "PUT",
        "/api/v1/buckets/nowhere/quota",
        Some(&root),
        Some(&set),
    )
    .await;
    assert_eq!(
        (missing.status, missing.code().as_deref()),
        (404, Some("no_such_bucket"))
    );

    let actions = send(&console, "GET", "/api/v1/actions", Some(&root), None).await;
    let recorded: Vec<(String, String, String, String)> = actions.json()["actions"]
        .as_array()
        .expect("actions")
        .iter()
        .filter(|action| action["operation"] == "set_quota")
        .map(|action| {
            let text = |field: &str| action[field].as_str().unwrap_or_default().to_owned();
            (
                text("target"),
                text("reason"),
                text("outcome"),
                text("operation"),
            )
        })
        .collect();
    assert_eq!(
        recorded.len(),
        3,
        "two settings and the missing bucket: {recorded:?}"
    );
    assert!(
        recorded
            .iter()
            .any(|(target, reason, outcome, _)| target == "media"
                && reason == "tenant plan"
                && outcome == "done"),
        "{recorded:?}"
    );
}
