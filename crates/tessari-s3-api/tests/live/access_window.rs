//! When a user's key stops working, against a real node: an unknown key and a wrong secret are refused at once; a
//! disabled key keeps working only while its cached resolution is younger than the 5-second window, measured on a
//! clock the test moves rather than one it waits for.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use tessari_s3_api::ApiState;
use tessari_s3_core::authz::Role;

use crate::access::user_with_key;
use crate::{IGNORED, call_as, live_config, prepared, test_node};

/// A state whose clock starts at the real time and moves only when the test says.
async fn state_with_clock() -> (ApiState, Arc<AtomicI64>) {
    let config = live_config(test_node(), None, None);
    let storage = prepared(&config).await;
    let start = ApiState::system_clock()();
    let now = Arc::new(AtomicI64::new(start));
    let read = Arc::clone(&now);
    let clock = Arc::new(move || read.load(Ordering::Acquire));
    (ApiState::new(&config, clock, storage), now)
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn an_unknown_key_or_a_wrong_secret_is_refused() {
    let _why = IGNORED;
    let (state, _) = state_with_clock().await;
    let ann = user_with_key(state.storage(), "ann", "alpha", Role::Member, true).await;
    let unknown = call_as(
        &state,
        ("TSNOSUCHKEYAAAAAAAAA", &ann.secret),
        "GET",
        "/",
        vec![],
        vec![],
        b"",
    )
    .await;
    assert_eq!(
        (unknown.status, unknown.code.as_deref()),
        (403, Some("InvalidAccessKeyId"))
    );
    let wrong = call_as(
        &state,
        (&ann.id, "not-the-secret-0123456789abcdefghijklm"),
        "GET",
        "/",
        vec![],
        vec![],
        b"",
    )
    .await;
    assert_eq!(
        (wrong.status, wrong.code.as_deref()),
        (403, Some("SignatureDoesNotMatch"))
    );
    let right = call_as(&state, ann.pair(), "GET", "/", vec![], vec![], b"").await;
    assert_eq!(right.status, 200);
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_disabled_key_is_refused_once_its_resolution_is_older_than_the_window() {
    let _why = IGNORED;
    let (state, now) = state_with_clock().await;
    let ann = user_with_key(state.storage(), "ann", "alpha", Role::Member, true).await;
    assert_eq!(
        call_as(&state, ann.pair(), "GET", "/", vec![], vec![], b"")
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
    let cached = call_as(&state, ann.pair(), "GET", "/", vec![], vec![], b"").await;
    assert_eq!(
        cached.status, 200,
        "within the window the cached resolution still answers"
    );
    now.fetch_add(4, Ordering::AcqRel);
    let still = call_as(&state, ann.pair(), "GET", "/", vec![], vec![], b"").await;
    assert_eq!(still.status, 200, "4 s after the resolution");
    now.fetch_add(1, Ordering::AcqRel);
    let refused = call_as(&state, ann.pair(), "GET", "/", vec![], vec![], b"").await;
    assert_eq!(
        (refused.status, refused.code.as_deref()),
        (403, Some("InvalidAccessKeyId")),
        "5 s after the resolution the key is looked up again"
    );
}
