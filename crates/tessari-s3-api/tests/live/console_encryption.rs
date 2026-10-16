//! Encryption keys through the console, against a real node: root sees whether they are held, unseals them with the
//! keys passphrase — a weak or wrong one refused by name — and seals them again; a user is refused; every act is
//! recorded with how it ended and never with the passphrase.

use serde_json::Value;
use tessari_s3_core::authz::Role;

use crate::access::user_with_key;
use crate::console::{send, sign_in};
use crate::console_users::sign_in_as;
use crate::{IGNORED, fresh_console};

const STATE: &str = "/api/v1/encryption";
const UNSEAL: &str = "/api/v1/encryption/unseal";
const SEAL: &str = "/api/v1/encryption/seal";
const RIGHT: &str = "the console keys passphrase";

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn root_unseals_and_seals_the_keys_and_every_act_is_recorded_without_the_passphrase() {
    let _why = IGNORED;
    let (state, console) = fresh_console().await;
    let root = sign_in(&console).await;
    let read = |cookie: String| {
        let console = console.clone();
        async move {
            send(&console, "GET", STATE, Some(&cookie), None)
                .await
                .json()
        }
    };
    assert_eq!(read(root.clone()).await["state"], "uninitialised");

    for (body, code) in [
        (r#"{"passphrase":"short"}"#.to_owned(), "weak_passphrase"),
        (format!(r#"{{"passphrase":"{RIGHT}'"}}"#), "weak_passphrase"),
    ] {
        let refused = send(&console, "POST", UNSEAL, Some(&root), Some(&body)).await;
        assert_eq!(
            (refused.status, refused.code().as_deref()),
            (400, Some(code)),
            "{body}"
        );
    }
    let body = format!(r#"{{"passphrase":"{RIGHT}"}}"#);
    let opened = send(&console, "POST", UNSEAL, Some(&root), Some(&body)).await;
    assert_eq!(opened.status, 200);
    assert_eq!(
        (
            opened.json()["state"].clone(),
            opened.json()["active"].clone()
        ),
        (Value::from("unsealed"), Value::from("server_1"))
    );
    assert!(
        state.storage().keys().ring().is_ok(),
        "the node holds the keys"
    );

    let sealed = send(
        &console,
        "POST",
        SEAL,
        Some(&root),
        Some(r#"{"reason":"drill"}"#),
    )
    .await;
    assert_eq!(sealed.status, 200);
    assert_eq!(read(root.clone()).await["state"], "sealed");
    assert!(state.storage().keys().ring().is_err());

    let wrong = r#"{"passphrase":"not the keys passphrase at all"}"#;
    let refused = send(&console, "POST", UNSEAL, Some(&root), Some(wrong)).await;
    assert_eq!(
        (refused.status, refused.code().as_deref()),
        (400, Some("wrong_passphrase"))
    );
    assert_eq!(read(root.clone()).await["state"], "sealed");

    let key = user_with_key(state.storage(), "enc", "enc-space", Role::SpaceAdmin, true).await;
    let user = sign_in_as(&console, &key).await.expect("signed in");
    for (path, body) in [
        (STATE, None),
        (UNSEAL, Some(body.as_str())),
        (SEAL, Some("{}")),
    ] {
        let method = if body.is_some() { "POST" } else { "GET" };
        let denied = send(&console, method, path, Some(&user), body).await;
        assert_eq!(denied.status, 403, "{path}");
    }
    assert!(
        state.storage().keys().ring().is_err(),
        "a user unsealed nothing"
    );

    let actions = send(&console, "GET", "/api/v1/actions", Some(&root), None).await;
    let text = String::from_utf8_lossy(&actions.bytes).into_owned();
    assert!(
        !text.contains(RIGHT) && !text.contains("not the keys"),
        "{text}"
    );
    let recorded: Vec<(String, String)> = actions.json()["actions"]
        .as_array()
        .expect("actions")
        .iter()
        .filter(|action| {
            action["operation"]
                .as_str()
                .is_some_and(|operation| operation.ends_with("_keys"))
        })
        .map(|action| {
            (
                action["operation"].as_str().unwrap_or_default().to_owned(),
                action["outcome"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    for expected in [
        ("unseal_keys", "unsealed"),
        ("seal_keys", "sealed"),
        ("unseal_keys", "wrong_passphrase"),
    ] {
        assert!(
            recorded.contains(&(expected.0.to_owned(), expected.1.to_owned())),
            "{expected:?} in {recorded:?}"
        );
    }
}

const DEFAULT: &str = "/api/v1/buckets/sealed-by-default/encryption";

/// The default encryption the bucket list shows for `bucket`.
fn default_of(listing: &Value, bucket: &str) -> Value {
    listing["buckets"]
        .as_array()
        .expect("buckets")
        .iter()
        .find(|row| row["name"] == bucket)
        .map(|row| row["encryption"].clone())
        .expect("listed")
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_bucket_default_is_set_only_by_who_configures_it_and_only_under_a_key_held() {
    let _why = IGNORED;
    let (state, console) = fresh_console().await;
    assert_eq!(
        crate::call(&state, "PUT", "/sealed-by-default", vec![], b"")
            .await
            .status,
        200
    );
    let root = sign_in(&console).await;
    let server = r#"{"algorithm":"AES256","reason":"tenant asked"}"#;
    let ann = user_with_key(state.storage(), "ann", "alpha", Role::Member, true).await;
    let as_ann = sign_in_as(&console, &ann).await.expect("signed in");
    let probe = send(&console, "PUT", DEFAULT, Some(&as_ann), Some(server)).await;
    assert_eq!(probe.status, 403, "a member who may not configure it");

    let sealed = send(&console, "PUT", DEFAULT, Some(&root), Some(server)).await;
    assert_eq!(
        (sealed.status, sealed.code().as_deref()),
        (409, Some("keys_sealed"))
    );
    crate::sse::unseal(&state).await;
    for (body, status, code) in [
        (
            r#"{"algorithm":"aws:kms","key":"nobody","reason":"x"}"#,
            400,
            "unknown_key",
        ),
        (
            r#"{"algorithm":"aws:kms","key":"Not A Name","reason":"x"}"#,
            400,
            "invalid_key_name",
        ),
        (
            r#"{"algorithm":"DES","reason":"x"}"#,
            400,
            "invalid_encryption",
        ),
        (r#"{"algorithm":"AES256"}"#, 400, "reason_required"),
    ] {
        let refused = send(&console, "PUT", DEFAULT, Some(&root), Some(body)).await;
        assert_eq!(
            (refused.status, refused.code().as_deref()),
            (status, Some(code)),
            "{body}"
        );
    }
    let listed = send(&console, "GET", "/api/v1/buckets", Some(&root), None).await;
    assert_eq!(
        default_of(&listed.json(), "sealed-by-default"),
        Value::Null,
        "nothing was set by a refusal"
    );

    let set = send(&console, "PUT", DEFAULT, Some(&root), Some(server)).await;
    assert_eq!(set.status, 204);
    let listed = send(&console, "GET", "/api/v1/buckets", Some(&root), None).await;
    assert_eq!(
        default_of(&listed.json(), "sealed-by-default")["algorithm"],
        "AES256"
    );
    let put = crate::call(&state, "PUT", "/sealed-by-default/k", vec![], b"bytes").await;
    assert_eq!(
        crate::sse::sse_header(&put),
        Some(&b"AES256"[..]),
        "the default seals a write set from the console"
    );

    let cleared = send(
        &console,
        "PUT",
        DEFAULT,
        Some(&root),
        Some(r#"{"algorithm":"none","reason":"tenant left"}"#),
    )
    .await;
    assert_eq!(cleared.status, 204);
    let listed = send(&console, "GET", "/api/v1/buckets", Some(&root), None).await;
    assert_eq!(default_of(&listed.json(), "sealed-by-default"), Value::Null);

    let actions = send(&console, "GET", "/api/v1/actions", Some(&root), None).await;
    let recorded: Vec<(String, String)> = actions.json()["actions"]
        .as_array()
        .expect("actions")
        .iter()
        .filter(|action| {
            action["operation"]
                .as_str()
                .is_some_and(|operation| operation.starts_with("set_encryption"))
        })
        .map(|action| {
            let text = |field: &str| action[field].as_str().unwrap_or_default().to_owned();
            (text("operation"), text("outcome"))
        })
        .collect();
    for expected in [
        ("set_encryption AES256", "sealed"),
        ("set_encryption aws:kms nobody", "unknown_key"),
        ("set_encryption AES256", "done"),
        ("set_encryption none", "done"),
    ] {
        assert!(
            recorded
                .iter()
                .any(|(operation, outcome)| (operation.as_str(), outcome.as_str()) == expected),
            "{expected:?} in {recorded:?}"
        );
    }
}
