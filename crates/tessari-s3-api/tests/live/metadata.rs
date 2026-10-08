//! The metadata store as a cluster sees it: the namespace states its replication (a cluster refuses one that does
//! not), a listed node that does not answer is passed over for the next, and a statement sent to a node that then
//! stops answering ends at the statement deadline instead of holding the request.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tessari_s3_infrastructure::S3Config;
use tessari_s3_infrastructure::tessaridb::{Answer, MetaError, MetaPool, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::{ACCESS_KEY, IGNORED, SECRET, call, fresh, fresh_at, need, test_node};

/// The namespace's own description, read through `USE` because the pool scopes every unit of work to it.
async fn namespace_info(planter: &MetaPool) -> Value {
    let (namespace, _) = planter.scope();
    let answers = planter
        .run_unscoped(&format!("USE NAMESPACE {namespace}; INFO FOR NAMESPACE;"))
        .await
        .expect("INFO FOR NAMESPACE answers");
    match answers.into_iter().nth(1) {
        Some(Answer::Value { value, .. }) => value,
        other => panic!("not a value: {other:?}"),
    }
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn the_schema_declares_the_namespaces_replication() {
    let _why = IGNORED;
    let (_, planter) = fresh().await;
    match namespace_info(&planter).await {
        Value::Object(fields) => assert_eq!(
            fields.get("replication"),
            Some(&Value::String("none".to_owned())),
            "declined out loud, not left unsaid: {fields:?}"
        ),
        other => panic!("not an object: {other:?}"),
    }
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_listed_metadata_node_that_does_not_answer_is_passed_over() {
    let _why = IGNORED;
    // Nothing listens on this port during the suite: hand-run nodes use 47961/47962.
    let addresses = format!("127.0.0.1:47909,{}", test_node());
    let (state, _, _) = fresh_at(addresses, Some("none"), None).await;
    assert_eq!(
        call(&state, "PUT", "/failover", vec![], b"").await.status,
        200
    );
    let listed = call(&state, "GET", "/", vec![], b"").await;
    assert!(
        listed.body.contains("<Name>failover</Name>"),
        "{}",
        listed.body
    );
}

/// A TCP relay in front of the test node that can be frozen: once frozen it keeps every connection open and
/// forwards nothing more, as a node partitioned away mid-statement does.
struct Relay {
    address: String,
    frozen: Arc<AtomicBool>,
    server: tokio::task::JoinHandle<()>,
}

impl Relay {
    async fn start(target: String) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free port");
        let address = listener.local_addr().expect("bound").to_string();
        let frozen = Arc::new(AtomicBool::new(false));
        let gate = Arc::clone(&frozen);
        let server = tokio::spawn(async move {
            while let Ok((client, _)) = listener.accept().await {
                let node = TcpStream::connect(&target)
                    .await
                    .expect("the test node answers");
                let (client_read, client_write) = client.into_split();
                let (node_read, node_write) = node.into_split();
                tokio::spawn(forward(client_read, node_write, Arc::clone(&gate)));
                tokio::spawn(forward(node_read, client_write, Arc::clone(&gate)));
            }
        });
        Self {
            address,
            frozen,
            server,
        }
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn forward(
    mut from: tokio::net::tcp::OwnedReadHalf,
    mut to: tokio::net::tcp::OwnedWriteHalf,
    frozen: Arc<AtomicBool>,
) {
    let mut buffer = vec![0_u8; 16 * 1024];
    loop {
        let Ok(read) = from.read(&mut buffer).await else {
            return;
        };
        if read == 0 {
            return;
        }
        if frozen.load(Ordering::SeqCst) {
            // Hold both halves open and say nothing.
            std::future::pending::<()>().await;
        }
        if to
            .write_all(buffer.get(..read).expect("read within the buffer"))
            .await
            .is_err()
        {
            return;
        }
    }
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_statement_whose_node_stops_answering_ends_at_the_statement_deadline() {
    let _why = IGNORED;
    let relay = Relay::start(test_node()).await;
    let vars = [
        ("TESSARIDB_S3_ROOT_ACCESS_KEY", ACCESS_KEY.to_owned()),
        ("TESSARIDB_S3_ROOT_SECRET_KEY", SECRET.to_owned()),
        ("TESSARIDB_S3_META_ADDRESS", relay.address.clone()),
        (
            "TESSARIDB_S3_META_USER",
            need("TESSARIDB_S3_TEST_META_USER"),
        ),
        (
            "TESSARIDB_S3_META_PASSWORD",
            need("TESSARIDB_S3_TEST_META_PASSWORD"),
        ),
        ("TESSARIDB_S3_META_STATEMENT_TIMEOUT_SECS", "2".to_owned()),
    ];
    let config =
        S3Config::from_lookup(|key| vars.iter().find(|(k, _)| *k == key).map(|(_, v)| v.clone()))
            .expect("configuration");
    let pool = MetaPool::new(config.meta).expect("pool");
    pool.run_unscoped("INFO FOR STORE;")
        .await
        .expect("answers while the relay forwards");
    relay.frozen.store(true, Ordering::SeqCst);
    let started = Instant::now();
    let ended = tokio::time::timeout(
        Duration::from_secs(10),
        pool.run_unscoped("INFO FOR STORE;"),
    )
    .await;
    let waited = started.elapsed();
    let Ok(answer) = ended else {
        panic!("the statement was still waiting after 10 s");
    };
    assert!(
        matches!(answer, Err(MetaError::Unavailable { .. })),
        "{answer:?}"
    );
    assert!(
        waited >= Duration::from_secs(2),
        "ended before the deadline: {waited:?}"
    );
}
