//! Delivering bucket notifications to a webhook against a real node (S3 ref 11 M5): the destination's group hands out
//! each event its bucket's rules send there, the worker posts it as an S3 event record, and a refused delivery is
//! handed out again later rather than lost — a receiver on this test's own port stands in for the destination.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use tessari_s3_daemons::{Daemon, NotifyWorker};
use tessari_s3_infrastructure::WebhookClient;

use crate::notifications::{CONFIGURATION, notification};
use crate::{IGNORED, call, fresh};

/// What the receiver saw: each delivery's key and event name, and the status it answers with.
#[derive(Clone, Default)]
struct Receiver {
    seen: Arc<Mutex<Vec<serde_json::Value>>>,
    status: Arc<AtomicU16>,
}

async fn receive(State(receiver): State<Receiver>, body: axum::body::Bytes) -> StatusCode {
    let record: serde_json::Value = serde_json::from_slice(&body).expect("a JSON body");
    receiver
        .seen
        .lock()
        .expect("not poisoned")
        .push(record["Records"][0].clone());
    StatusCode::from_u16(receiver.status.load(Ordering::SeqCst)).expect("a status")
}

/// A receiver on an ephemeral port; its URL.
async fn listening(receiver: Receiver) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a port");
    let address = listener.local_addr().expect("an address");
    let app = Router::new()
        .route("/hooks", post(receive))
        .with_state(receiver);
    tokio::spawn(async move { axum::serve(listener, app).await });
    format!("http://{address}/hooks")
}

/// Deliveries the receiver saw per key, in arrival order.
fn by_key(receiver: &Receiver) -> BTreeMap<String, Vec<String>> {
    let mut keys: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for record in receiver.seen.lock().expect("not poisoned").iter() {
        let key = record["s3"]["object"]["key"]
            .as_str()
            .expect("a key")
            .to_owned();
        let name = record["eventName"].as_str().expect("a name").to_owned();
        keys.entry(key).or_default().push(name);
    }
    keys
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn matching_events_reach_the_webhook_once_and_a_refused_one_is_retried() {
    let _why = IGNORED;
    let (state, _planter) = fresh().await;
    assert_eq!(
        call(&state, "PUT", "/hooked", vec![], b"").await.status,
        200
    );
    assert_eq!(
        notification(&state, "PUT", "hooked", CONFIGURATION)
            .await
            .status,
        200
    );
    // Written before the destination's group exists: a new group starts at the oldest message, so it is delivered.
    call(&state, "PUT", "/hooked/logs/early.txt", vec![], b"early").await;
    let receiver = Receiver::default();
    receiver.status.store(200, Ordering::SeqCst);
    let url = listening(receiver.clone()).await;
    let notifications = state.storage().notifications();
    notifications
        .define(["hooks"])
        .await
        .expect("group defined");
    let worker = NotifyWorker::new(
        notifications,
        "hooks".to_owned(),
        url,
        "us-east-1".to_owned(),
        WebhookClient::new().expect("a client"),
    );
    call(&state, "PUT", "/hooked/logs/a.txt", vec![], b"a").await;
    // No rule names this key: settled without a delivery.
    call(&state, "PUT", "/hooked/other.bin", vec![], b"b").await;
    worker.run_once().await.expect("a run");
    let first = by_key(&receiver);
    assert_eq!(
        first,
        BTreeMap::from([
            (
                "logs/a.txt".to_owned(),
                vec!["ObjectCreated:Put".to_owned()]
            ),
            (
                "logs/early.txt".to_owned(),
                vec!["ObjectCreated:Put".to_owned()]
            ),
        ])
    );
    worker.run_once().await.expect("a second run");
    assert_eq!(
        by_key(&receiver),
        first,
        "a delivered event is not delivered again"
    );

    // The receiver refuses: the event is handed out again after the retry wait, and delivered then.
    receiver.status.store(503, Ordering::SeqCst);
    call(&state, "PUT", "/hooked/logs/b.txt", vec![], b"b").await;
    worker.run_once().await.expect("a refused run");
    receiver.status.store(200, Ordering::SeqCst);
    let mut attempts = 0;
    for _ in 0..40 {
        worker.run_once().await.expect("a retrying run");
        attempts = by_key(&receiver).get("logs/b.txt").map_or(0, Vec::len);
        if attempts == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert_eq!(attempts, 2, "refused once, then delivered");
    worker.run_once().await.expect("a settled run");
    assert_eq!(
        by_key(&receiver).get("logs/b.txt").map(Vec::len),
        Some(2),
        "delivered, then settled"
    );
}
