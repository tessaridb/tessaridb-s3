//! Bucket notifications survive the nodes delivering them (G006 C3): every node is killed while a delivery is in
//! flight, and once they are back every event still reaches the destination — at least once, none lost.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use tessari_s3_infrastructure::tessaridb::Value;

use crate::requests::call;
use crate::s3::{Cluster, Meta};

/// How long redelivery may take after the restart: the group's ack deadline (60 s) and a margin.
const DELIVERED_WITHIN: Duration = Duration::from_secs(150);

/// The destination: holds every delivery open while `hold` is set, answers 200 otherwise; records each key it saw.
#[derive(Clone, Default)]
struct Receiver {
    hold: Arc<AtomicBool>,
    attempted: Arc<Mutex<Vec<String>>>,
    delivered: Arc<Mutex<BTreeSet<String>>>,
}

async fn receive(State(receiver): State<Receiver>, body: axum::body::Bytes) -> StatusCode {
    let record: serde_json::Value = serde_json::from_slice(&body).expect("a JSON body");
    let key = record["Records"][0]["s3"]["object"]["key"]
        .as_str()
        .expect("a key")
        .to_owned();
    receiver
        .attempted
        .lock()
        .expect("not poisoned")
        .push(key.clone());
    if receiver.hold.load(Ordering::SeqCst) {
        // In flight until the node holding it dies and drops the connection.
        std::future::pending::<()>().await;
    }
    receiver.delivered.lock().expect("not poisoned").insert(key);
    StatusCode::OK
}

pub async fn a_delivery_cut_off_by_killed_nodes_is_made_after_they_return(meta: Meta) {
    let receiver = Receiver::default();
    receiver.hold.store(true, Ordering::SeqCst);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a port");
    let address = listener.local_addr().expect("an address");
    let app = Router::new()
        .route("/hooks", post(receive))
        .with_state(receiver.clone());
    tokio::spawn(async move { axum::serve(listener, app).await });
    let meta = Meta {
        notify_targets: Some(format!("hooks=webhook:http://{address}/hooks")),
        ..meta
    };
    let mut cluster = Cluster::start(meta).await;
    assert_eq!(call(&cluster.nodes[0], "PUT", "/told", b"").await.0, 200);
    cluster
        .meta
        .run(
            "UPDATE buckets:'told' SET notify = { rules: [{ id: 'all', target: 'hooks', \
             events: ['s3:ObjectCreated:*'], prefix: '', suffix: '' }] };",
            Vec::<(String, Value)>::new(),
        )
        .await
        .expect("configured");
    let keys: BTreeSet<String> = (0..5).map(|n| format!("k{n}")).collect();
    for key in &keys {
        assert_eq!(
            call(&cluster.nodes[0], "PUT", &format!("/told/{key}"), b"bytes")
                .await
                .0,
            200
        );
    }
    // A readiness signal, not a timing assertion: wait until some node is mid-delivery.
    let started = Instant::now();
    while receiver.attempted.lock().expect("not poisoned").is_empty() {
        assert!(
            started.elapsed() < DELIVERED_WITHIN,
            "no delivery began; see {}",
            cluster.logs.display()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    for node in &mut cluster.nodes {
        node.child.kill().await.expect("killed");
    }
    receiver.hold.store(false, Ordering::SeqCst);
    for index in 0..cluster.nodes.len() {
        cluster.restart(index).await;
    }
    let restarted = Instant::now();
    loop {
        let delivered = receiver.delivered.lock().expect("not poisoned").clone();
        if delivered == keys {
            break;
        }
        assert!(
            restarted.elapsed() < DELIVERED_WITHIN,
            "delivered {delivered:?} of {keys:?}; see {}",
            cluster.logs.display()
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}
