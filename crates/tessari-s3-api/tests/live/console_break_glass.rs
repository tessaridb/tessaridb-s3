//! Signing in with the root credential where an identity provider is configured is the emergency path, and it is made
//! deliberate and loud: it owes a reason, its session lasts a quarter of an hour, it is in the action record before the
//! session exists, and it is announced to the configured webhook as it happens. A user's key is not affected.

use std::sync::{Arc, Mutex};

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use serde_json::Value;

use crate::console::{Seen, send};
use crate::console_oidc::{console_with, consoles_with};
use crate::oidc_provider::Provider;
use crate::users::user;
use crate::{ACCESS_KEY, IGNORED, SECRET};
use tessari_s3_infrastructure::KeySignIn;

/// A webhook on a loopback port that keeps what it is sent; `/down` answers every alert with a 500.
struct Receiver {
    url: String,
    received: Arc<Mutex<Vec<Value>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Receiver {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn keep(
    State(received): State<Arc<Mutex<Vec<Value>>>>,
    Json(alert): Json<Value>,
) -> StatusCode {
    received.lock().expect("received").push(alert);
    StatusCode::NO_CONTENT
}

async fn refuse() -> StatusCode {
    StatusCode::INTERNAL_SERVER_ERROR
}

impl Receiver {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a loopback port");
        let url = format!("http://{}", listener.local_addr().expect("address"));
        let received = Arc::<Mutex<Vec<Value>>>::default();
        let app = axum::Router::new()
            .route("/alert", post(keep))
            .route("/down", post(refuse))
            .with_state(Arc::clone(&received));
        let task = tokio::spawn(async move {
            let _ended = axum::serve(listener, app).await;
        });
        Self {
            url,
            received,
            task,
        }
    }

    fn alerts(&self) -> Vec<Value> {
        self.received.lock().expect("received").clone()
    }
}

fn root(reason: Option<&str>) -> String {
    let reason = reason.map_or_else(String::new, |why| format!(",\"reason\":\"{why}\""));
    format!("{{\"access_key_id\":\"{ACCESS_KEY}\",\"secret_access_key\":\"{SECRET}\"{reason}}}")
}

fn max_age(seen: &Seen) -> Option<&str> {
    seen.header("set-cookie")?
        .split("; ")
        .find_map(|part| part.strip_prefix("Max-Age="))
}

async fn operations(console: &tessari_s3_api::ConsoleState, token: &str) -> Vec<Value> {
    send(console, "GET", "/api/v1/actions", Some(token), None)
        .await
        .json()["actions"]
        .as_array()
        .expect("actions")
        .iter()
        .filter(|action| action["operation"] == "break_glass_sign_in")
        .cloned()
        .collect()
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn root_beside_a_provider_owes_a_reason_lasts_a_quarter_hour_and_is_announced_and_recorded() {
    let _why = IGNORED;
    let provider = Provider::start().await;
    let receiver = Receiver::start().await;
    let (console, storage) = console_with(&provider, Some(format!("{}/alert", receiver.url))).await;

    let bare = send(&console, "POST", "/api/v1/session", None, Some(&root(None))).await;
    assert_eq!(bare.status, 400);
    assert_eq!(bare.code().as_deref(), Some("reason_required"));
    assert!(bare.cookie().is_none(), "no session without a reason");
    assert!(
        receiver.alerts().is_empty(),
        "nothing announced for a refusal"
    );

    let signed = send(
        &console,
        "POST",
        "/api/v1/session",
        None,
        Some(&root(Some("ticket 7: provider down"))),
    )
    .await;
    assert_eq!(signed.status, 200);
    assert_eq!(max_age(&signed), Some("900"));
    let token = signed.cookie().expect("a session");
    let alerts = receiver.alerts();
    assert_eq!(alerts.len(), 1, "{alerts:?}");
    assert_eq!(alerts[0]["event"], "break_glass_sign_in");
    assert_eq!(alerts[0]["key_id"], ACCESS_KEY);
    assert_eq!(alerts[0]["reason"], "ticket 7: provider down");
    assert_eq!(alerts[0]["client"], "10.0.0.9");
    assert_eq!(alerts[0]["expires"], signed.json()["expires"]);
    let recorded = operations(&console, &token).await;
    assert_eq!(recorded.len(), 1, "{recorded:?}");
    assert_eq!(recorded[0]["operator"], ACCESS_KEY);
    assert_eq!(recorded[0]["reason"], "ticket 7: provider down");
    assert_eq!(recorded[0]["outcome"], "done");

    // A user's own key is the ordinary path: no reason, an ordinary session, nothing announced.
    let issued = storage
        .users()
        .issue_key(&user("ann"))
        .await
        .expect("issued")
        .expect("ann");
    let ann = send(
        &console,
        "POST",
        "/api/v1/session",
        None,
        Some(&format!(
            "{{\"access_key_id\":\"{}\",\"secret_access_key\":\"{}\"}}",
            issued.access_key_id,
            issued.secret.expose()
        )),
    )
    .await;
    assert_eq!(ann.status, 200);
    assert_eq!(max_age(&ann), Some("3600"));
    assert_eq!(
        receiver.alerts().len(),
        1,
        "a user's sign-in is not announced"
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn an_alert_that_fails_does_not_bar_the_emergency_path_and_the_record_says_so() {
    let _why = IGNORED;
    let provider = Provider::start().await;
    let receiver = Receiver::start().await;
    let (console, _) = console_with(&provider, Some(format!("{}/down", receiver.url))).await;
    let signed = send(
        &console,
        "POST",
        "/api/v1/session",
        None,
        Some(&root(Some("ticket 8"))),
    )
    .await;
    assert_eq!(signed.status, 200);
    let token = signed.cookie().expect("a session");
    let recorded = operations(&console, &token).await;
    assert_eq!(recorded.len(), 1, "{recorded:?}");
    assert_eq!(recorded[0]["outcome"], "alert_failed");
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn with_key_sign_in_set_to_root_a_users_key_opens_no_console_session_and_root_still_does() {
    let _why = IGNORED;
    let provider = Provider::start().await;
    let (consoles, storage) =
        consoles_with(&provider, None, &[KeySignIn::All, KeySignIn::Root]).await;
    let (all, root_only) = (&consoles[0], &consoles[1]);
    let issued = storage
        .users()
        .issue_key(&user("ann"))
        .await
        .expect("issued")
        .expect("ann");
    let ann = |secret: &str| {
        format!(
            "{{\"access_key_id\":\"{}\",\"secret_access_key\":\"{secret}\"}}",
            issued.access_key_id
        )
    };
    let methods = send(root_only, "GET", "/api/v1/auth/methods", None, None).await;
    assert_eq!(methods.json()["keys"], Value::from("root"));
    let before = send(
        all,
        "POST",
        "/api/v1/session",
        None,
        Some(&ann(issued.secret.expose())),
    )
    .await;
    assert_eq!(before.status, 200);
    let earlier = before.cookie().expect("a session under all");

    let refused = send(
        root_only,
        "POST",
        "/api/v1/session",
        None,
        Some(&ann(issued.secret.expose())),
    )
    .await;
    assert_eq!(refused.status, 403);
    assert_eq!(refused.code().as_deref(), Some("key_sign_in_disabled"));
    assert!(refused.cookie().is_none(), "no session for a user's key");
    // The credential is checked first: a wrong secret is the ordinary refusal and says nothing about the setting.
    let wrong = send(
        root_only,
        "POST",
        "/api/v1/session",
        None,
        Some(&ann("not-the-secret")),
    )
    .await;
    assert_eq!(wrong.status, 401);
    // A session a user's key opened before the switch ends at its next request.
    let carried = send(root_only, "GET", "/api/v1/session", Some(&earlier), None).await;
    assert_eq!(carried.status, 401);

    let emergency = send(
        root_only,
        "POST",
        "/api/v1/session",
        None,
        Some(&root(Some("ticket 9"))),
    )
    .await;
    assert_eq!(emergency.status, 200);
    assert_eq!(max_age(&emergency), Some("900"));
}
