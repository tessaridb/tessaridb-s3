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
