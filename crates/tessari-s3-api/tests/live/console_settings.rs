//! Stored settings through the console, against a real node: root lists them — a secret by name only — and changes
//! one against the value it read, with a reason; a stale read, an invalid value, a fixed setting and a secret without
//! the passphrase are refused by name; a user is refused; every attempt is recorded with the setting's name and never
//! its value.

use std::sync::Arc;

use serde_json::Value;
use tessari_s3_api::SettingsCheck;
use tessari_s3_core::authz::Role;
use tessari_s3_infrastructure::S3Config;

use crate::access::user_with_key;
use crate::console::{record, send, sign_in};
use crate::console_users::sign_in_as;
use crate::{ACCESS_KEY, IGNORED, SECRET, fresh_console};

const LIST: &str = "/api/v1/settings";
const RECLAIM: &str = "/api/v1/settings/TESSARIDB_S3_RECLAIM_INTERVAL_SECS";
const IAM: &str = "/api/v1/settings/TESSARIDB_S3_IAM_KEY";
const PASSPHRASE: &str = "the settings keys passphrase";

/// A check that accepts what a minimal configuration accepts with the one value in place.
fn check() -> SettingsCheck {
    Arc::new(|name: &str, value: &str| {
        let base = [
            ("TESSARIDB_S3_ROOT_ACCESS_KEY", ACCESS_KEY),
            ("TESSARIDB_S3_ROOT_SECRET_KEY", SECRET),
            ("TESSARIDB_S3_META_ADDRESS", "127.0.0.1:1"),
            ("TESSARIDB_S3_META_USER", "s3"),
            ("TESSARIDB_S3_META_PASSWORD", "a password of length"),
            ("TESSARIDB_S3_DATA_DIR", "/tmp/unused"),
        ];
        S3Config::from_lookup(|key| {
            if key == name {
                Some(value.to_owned())
            } else {
                base.iter()
                    .find(|(known, _)| *known == key)
                    .map(|(_, given)| (*given).to_owned())
            }
        })
        .map(|_| ())
    })
}

fn row<'a>(listed: &'a Value, name: &str) -> &'a Value {
    listed["settings"]
        .as_array()
        .expect("settings")
        .iter()
        .find(|setting| setting["name"] == name)
        .expect("listed")
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn root_changes_a_stored_setting_against_what_it_read_and_every_attempt_is_recorded() {
    let _why = IGNORED;
    let (state, console) = fresh_console().await;
    let root = sign_in(&console).await;
    let put = |path: &'static str, body: String| {
        let console = console.clone();
        let root = root.clone();
        async move { send(&console, "PUT", path, Some(&root), Some(&body)).await }
    };
    // Without a node to check values the console changes nothing.
    let unmanaged = put(RECLAIM, r#"{"value":"45","reason":"tune"}"#.to_owned()).await;
    assert_eq!(
        (unmanaged.status, unmanaged.code().as_deref()),
        (403, Some("settings_unmanaged"))
    );
    assert!(console.accept_settings_check(check()));

    let listed = send(&console, "GET", LIST, Some(&root), None).await;
    assert_eq!(listed.status, 200);
    let listed = listed.json();
    assert_eq!(listed["region"], "us-east-1");
    let reclaim = row(&listed, "TESSARIDB_S3_RECLAIM_INTERVAL_SECS");
    assert_eq!(
        (&reclaim["class"], &reclaim["applies"], &reclaim["stored"]),
        (
            &Value::from("setting"),
            &Value::from("live"),
            &Value::from(false)
        )
    );
    assert_eq!(row(&listed, "TESSARIDB_S3_ERASURE")["fixed"], true);
    assert_eq!(row(&listed, "TESSARIDB_S3_IAM_KEY")["class"], "secret");

    let unreasoned = put(RECLAIM, r#"{"value":"45"}"#.to_owned()).await;
    assert_eq!(unreasoned.code().as_deref(), Some("reason_required"));
    let changed = put(RECLAIM, r#"{"value":"45","reason":"tune"}"#.to_owned()).await;
    assert_eq!(changed.status, 200, "{:?}", changed.json());
    assert_eq!(changed.json()["applies"], "live");
    // The same change made against the absence it read is now stale.
    let stale = put(RECLAIM, r#"{"value":"46","reason":"again"}"#.to_owned()).await;
    assert_eq!(
        (stale.status, stale.code().as_deref()),
        (409, Some("setting_changed"))
    );
    let invalid = put(
        RECLAIM,
        r#"{"value":"often","expected":"45","reason":"typo"}"#.to_owned(),
    )
    .await;
    assert_eq!(
        (invalid.status, invalid.code().as_deref()),
        (400, Some("setting_invalid"))
    );
    let fixed = put(
        "/api/v1/settings/TESSARIDB_S3_ERASURE",
        r#"{"value":"4+2","reason":"grow"}"#.to_owned(),
    )
    .await;
    assert_eq!(
        (fixed.status, fixed.code().as_deref()),
        (409, Some("setting_fixed"))
    );
    let unknown = put(
        "/api/v1/settings/TESSARIDB_S3_LISTEN",
        r#"{"value":"0.0.0.0:1","reason":"no"}"#.to_owned(),
    )
    .await;
    assert_eq!(unknown.status, 404);
    let key = "ab".repeat(32);
    let sealed = put(IAM, format!(r#"{{"value":"{key}","reason":"rotate"}}"#)).await;
    assert_eq!(
        (sealed.status, sealed.code().as_deref()),
        (400, Some("passphrase_required"))
    );
    let rotated = put(
        IAM,
        format!(r#"{{"value":"{key}","reason":"rotate","passphrase":"{PASSPHRASE}"}}"#),
    )
    .await;
    assert_eq!(rotated.status, 200, "{:?}", rotated.json());
    assert_eq!(rotated.json()["applies"], "start");

    let listed = send(&console, "GET", LIST, Some(&root), None).await.json();
    assert_eq!(
        row(&listed, "TESSARIDB_S3_RECLAIM_INTERVAL_SECS")["value"],
        "45"
    );
    let iam = row(&listed, "TESSARIDB_S3_IAM_KEY");
    assert_eq!(
        (&iam["stored"], &iam["value"]),
        (&Value::from(true), &Value::Null)
    );
    assert!(
        !listed.to_string().contains(&key),
        "a secret is never shown"
    );

    // A user of a space may neither read nor change them.
    let user = user_with_key(state.storage(), "set", "set-space", Role::SpaceAdmin, true).await;
    let token = sign_in_as(&console, &user).await.expect("signed in");
    let read = send(&console, "GET", LIST, Some(&token), None).await;
    let write = send(
        &console,
        "PUT",
        RECLAIM,
        Some(&token),
        Some(r#"{"value":"1","expected":"45","reason":"mine"}"#),
    )
    .await;
    assert_eq!((read.status, write.status), (403, 403));

    let actions: Vec<_> = record(&console, &root)
        .await
        .into_iter()
        .filter(|(operation, ..)| operation == "change_setting")
        .map(|(_, target, reason, outcome)| (target, reason, outcome))
        .collect();
    let target = |name: &str| format!("setting:TESSARIDB_S3_{name}");
    let reason = |text: &str| Value::String(text.to_owned());
    assert_eq!(
        actions,
        [
            (target("IAM_KEY"), reason("rotate"), "changed".to_owned()),
            (
                target("IAM_KEY"),
                reason("rotate"),
                "passphrase_required".to_owned()
            ),
            (target("ERASURE"), reason("grow"), "fixed".to_owned()),
            (
                target("RECLAIM_INTERVAL_SECS"),
                reason("typo"),
                "invalid".to_owned()
            ),
            (
                target("RECLAIM_INTERVAL_SECS"),
                reason("again"),
                "raced".to_owned()
            ),
            (
                target("RECLAIM_INTERVAL_SECS"),
                reason("tune"),
                "changed".to_owned()
            ),
            (
                target("RECLAIM_INTERVAL_SECS"),
                reason("tune"),
                "unmanaged".to_owned()
            ),
        ]
    );
}
