//! A bucket's media rules through the console, against a real node (G004 C1): whoever configures the bucket sets them,
//! anyone who reads it sees them, a rule the server cannot honour is refused with nothing stored, and an empty list
//! turns media processing off.

use serde_json::{Value, json};
use tessari_s3_core::authz::Role;

use crate::access::user_with_key;
use crate::console::send;
use crate::console_users::{sign_in_as, states};
use crate::{IGNORED, call_as};

const MEDIA: &str = "/api/v1/buckets/pics/media";

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn the_bucket_s_configurer_sets_media_rules_and_a_bad_rule_is_refused() {
    let _why = IGNORED;
    let (state, console, _) = states().await;
    let ann = user_with_key(state.storage(), "ann", "alpha", Role::Member, true).await;
    let bob = user_with_key(state.storage(), "bob", "alpha", Role::Member, false).await;
    assert_eq!(
        call_as(&state, ann.pair(), "PUT", "/pics", vec![], vec![], b"")
            .await
            .status,
        200
    );
    let ann_token = sign_in_as(&console, &ann).await.expect("ann signed in");
    let bob_token = sign_in_as(&console, &bob).await.expect("bob signed in");
    let none = send(&console, "GET", MEDIA, Some(&ann_token), None).await;
    assert_eq!(none.status, 200);
    assert_eq!(none.json()["rules"], json!([]));

    let rules = json!({"rules": [
        {"id": "photos", "content_types": ["image/*"], "prefix": "photos/", "outputs": ["thumb", "web"]},
        {"id": "clips", "content_types": ["video/mp4"], "prefix": "", "outputs": ["720p", "poster"]}
    ]});
    let refused = send(
        &console,
        "PUT",
        MEDIA,
        Some(&bob_token),
        Some(&rules.to_string()),
    )
    .await;
    assert_eq!(
        refused.status, 403,
        "a member without a grant does not configure the bucket"
    );
    let set = send(
        &console,
        "PUT",
        MEDIA,
        Some(&ann_token),
        Some(&rules.to_string()),
    )
    .await;
    assert_eq!(set.status, 204, "{:?}", set.code());
    let got = send(&console, "GET", MEDIA, Some(&ann_token), None).await;
    assert_eq!(got.json()["rules"], rules["rules"]);

    for (body, code) in [
        (
            json!({"rules": [{"id": "a", "content_types": ["image/png"], "prefix": "", "outputs": ["720p"]}]}),
            "invalid_media",
        ),
        (
            json!({"rules": [{"id": "a", "content_types": ["image/png"], "prefix": "", "outputs": ["gif"]}]}),
            "invalid_media",
        ),
        (json!({"rules": [], "space": "beta"}), "bad_request"),
    ] {
        let seen = send(
            &console,
            "PUT",
            MEDIA,
            Some(&ann_token),
            Some(&body.to_string()),
        )
        .await;
        assert_eq!(seen.status, 400, "{body}");
        assert_eq!(seen.code().as_deref(), Some(code), "{body}");
    }
    assert_eq!(
        send(&console, "GET", MEDIA, Some(&ann_token), None)
            .await
            .json()["rules"],
        rules["rules"],
        "nothing refused was stored"
    );
    let off = send(
        &console,
        "PUT",
        MEDIA,
        Some(&ann_token),
        Some(&json!({"rules": []}).to_string()),
    )
    .await;
    assert_eq!(off.status, 204);
    let cleared: Value = send(&console, "GET", MEDIA, Some(&ann_token), None)
        .await
        .json();
    assert_eq!(cleared["rules"], json!([]));
}
