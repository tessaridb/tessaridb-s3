//! Six `tessaridb-s3` processes forming one RS(4, 2) cluster over a fresh TessariDB namespace, driven over real HTTP:
//! an object written through one node's S3 surface lands as one shard on every node; with two nodes killed it is
//! still acknowledged and listed for healing; with three killed nothing is acknowledged or committed.
//!
//! Needs `TESSARIDB_S3_TEST_META` (a node's wire `host:port`), `TESSARIDB_S3_TEST_META_USER` and
//! `TESSARIDB_S3_TEST_META_PASSWORD`; without them the test is `ignored`, and
//! `cargo test -p tessari-s3-node --test processes -- --ignored` runs it.

#![cfg(test)]

#[path = "../../tessari-s3-api/tests/wire/signer.rs"]
#[allow(
    dead_code,
    reason = "shared with the api crate's tests; this binary uses part of it"
)]
mod signer;

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use tessari_s3_infrastructure::S3Config;
use tessari_s3_infrastructure::tessaridb::{Answer, MetaPool};
use tessari_s3_types::Timestamp;
use tokio::process::{Child, Command};

const ACCESS_KEY: &str = "AKPROCESSTEST0000001";
const SECRET: &str = "process-test-secret-0123456789abcdef";
const CLUSTER_SECRET: &str = "process-cluster-secret-0123456789abcd";
/// How long one process may take to open its S3 port: schema, registration and both listeners.
const READY_WITHIN: Duration = Duration::from_secs(30);

fn need(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| panic!("{key} is required for this test"))
}

/// A free local port, released for the child to bind.
fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port");
    listener.local_addr().expect("bound").port()
}

/// One running node: its process, its S3 address and its data directory.
struct Node {
    child: Child,
    s3: String,
    dir: PathBuf,
}

/// The six nodes, the namespace they share and a pool into it; every process is killed and every directory removed
/// when it is dropped.
struct Cluster {
    nodes: Vec<Node>,
    meta: MetaPool,
    logs: PathBuf,
}

impl Drop for Cluster {
    fn drop(&mut self) {
        for node in &mut self.nodes {
            let _gone = node.child.start_kill();
            std::fs::remove_dir_all(&node.dir).ok();
        }
        std::fs::remove_dir_all(&self.logs).ok();
    }
}

fn scratch(kind: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "tessari-s3-{kind}-{}",
        uuid::Uuid::new_v4().simple()
    ))
}

/// The variables every node shares, plus this node's own.
fn environment(
    namespace: &str,
    node: &str,
    s3: &str,
    internal: &str,
    dir: &Path,
) -> Vec<(String, String)> {
    [
        ("TESSARIDB_S3_LISTEN", s3.to_owned()),
        ("TESSARIDB_S3_ROOT_ACCESS_KEY", ACCESS_KEY.to_owned()),
        ("TESSARIDB_S3_ROOT_SECRET_KEY", SECRET.to_owned()),
        ("TESSARIDB_S3_META_ADDRESS", need("TESSARIDB_S3_TEST_META")),
        (
            "TESSARIDB_S3_META_USER",
            need("TESSARIDB_S3_TEST_META_USER"),
        ),
        (
            "TESSARIDB_S3_META_PASSWORD",
            need("TESSARIDB_S3_TEST_META_PASSWORD"),
        ),
        ("TESSARIDB_S3_META_NAMESPACE", namespace.to_owned()),
        ("TESSARIDB_S3_DATA_DIR", dir.display().to_string()),
        ("TESSARIDB_S3_ERASURE", "4+2".to_owned()),
        ("TESSARIDB_S3_NODE_ID", node.to_owned()),
        ("TESSARIDB_S3_INTERNAL_LISTEN", internal.to_owned()),
        ("TESSARIDB_S3_INTERNAL_ADVERTISE", internal.to_owned()),
        ("TESSARIDB_S3_CLUSTER_SECRET", CLUSTER_SECRET.to_owned()),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value))
    .collect()
}

/// Starts node `name` and waits until its S3 port accepts, which the binary opens only once it is registered.
async fn start(namespace: &str, name: &str, logs: &Path) -> Node {
    let s3 = format!("127.0.0.1:{}", free_port());
    let internal = format!("127.0.0.1:{}", free_port());
    let dir = scratch("data");
    let log = std::fs::File::create(logs.join(format!("{name}.log"))).expect("log file");
    let child = Command::new(env!("CARGO_BIN_EXE_tessaridb-s3"))
        .env_clear()
        .envs(environment(namespace, name, &s3, &internal, &dir))
        .stdout(Stdio::from(log.try_clone().expect("log handle")))
        .stderr(Stdio::from(log))
        .kill_on_drop(true)
        .spawn()
        .expect("the binary starts");
    let started = Instant::now();
    // A readiness signal, not a timing assertion: the port is polled until the process opens it or the deadline.
    while tokio::net::TcpStream::connect(&s3).await.is_err() {
        assert!(
            started.elapsed() < READY_WITHIN,
            "{name} did not open its S3 port within {READY_WITHIN:?}; see {}",
            logs.display()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Node { child, s3, dir }
}

async fn cluster() -> Cluster {
    let namespace = format!("t_{}", uuid::Uuid::new_v4().simple());
    let logs = scratch("logs");
    std::fs::create_dir_all(&logs).expect("log directory");
    let mut nodes = Vec::new();
    // One after another: every node applies the schema, and concurrent applies conflict on the catalog (Q-S3-2).
    for n in 1..=6 {
        nodes.push(start(&namespace, &format!("n{n}"), &logs).await);
    }
    let vars = environment(
        &namespace,
        "n1",
        "127.0.0.1:1",
        "127.0.0.1:1",
        Path::new("/"),
    );
    let config = S3Config::from_lookup(|key| {
        vars.iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.clone())
    })
    .expect("configuration");
    Cluster {
        nodes,
        meta: MetaPool::new(config.meta).expect("pool"),
        logs,
    }
}

/// `YYYYMMDDTHHMMSSZ` for now.
fn amz_now() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_secs()).expect("seconds fit"))
        .expect("after the epoch");
    let iso = Timestamp { seconds, nanos: 0 }.iso8601_millis();
    format!(
        "{}{}{}T{}{}{}Z",
        &iso[0..4],
        &iso[5..7],
        &iso[8..10],
        &iso[11..13],
        &iso[14..16],
        &iso[17..19]
    )
}

/// Sends one signed request to `node` and answers its status and body.
async fn call(node: &Node, method: &str, path: &str, body: &[u8]) -> (u16, String) {
    let amz_date = amz_now();
    let unsigned = signer::Unsigned {
        method,
        host: &node.s3,
        path,
        query: Vec::new(),
        headers: Vec::new(),
        access_key: ACCESS_KEY,
        secret: SECRET,
        region: "us-east-1",
        amz_date: &amz_date,
    };
    let (target, headers) = signer::sign_with_body(&unsigned, body);
    let client = reqwest::Client::new();
    let mut request = client.request(
        reqwest::Method::from_bytes(method.as_bytes()).expect("method"),
        format!("http://{}{target}", node.s3),
    );
    for (name, value) in headers.iter().filter(|(name, _)| name != "host") {
        request = request.header(name.as_str(), value.as_str());
    }
    let response = request
        .body(body.to_vec())
        .timeout(Duration::from_secs(120))
        .send()
        .await
        .expect("the node answers");
    let status = response.status().as_u16();
    (status, response.text().await.unwrap_or_default())
}

/// Shard files under `dir`.
fn shard_files(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| {
            let path = entry.path();
            if path.is_dir() {
                shard_files(&path)
            } else {
                usize::from(path.to_string_lossy().contains(".s"))
            }
        })
        .sum()
}

async fn count(pool: &MetaPool, table: &str) -> usize {
    // The table name is this test's own literal, never a value from outside.
    let script = format!("SELECT * FROM {table};");
    match pool
        .run(&script, Vec::new())
        .await
        .expect("read")
        .into_iter()
        .next()
    {
        Some(Answer::Records { records, .. }) => records.len(),
        _ => 0,
    }
}

/// 300 KiB: past the inline size, inside one stripe.
fn large() -> Vec<u8> {
    (0..300 * 1024_u32)
        .map(|i| u8::try_from(i % 251).expect("small"))
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn six_processes_spread_an_object_and_hold_the_write_quorum() {
    let mut cluster = cluster().await;
    let front = 0;
    assert_eq!(
        call(&cluster.nodes[front], "PUT", "/shards", b"").await.0,
        200
    );
    let (status, body) = call(&cluster.nodes[front], "PUT", "/shards/first", &large()).await;
    assert_eq!(status, 200, "{body}");
    let spread: Vec<usize> = cluster
        .nodes
        .iter()
        .map(|node| shard_files(&node.dir))
        .collect();
    assert_eq!(spread, vec![1; 6], "one shard on every node");
    assert_eq!(
        count(&cluster.meta, "heals").await,
        0,
        "every shard durable"
    );

    for down in [2, 4] {
        cluster.nodes[down].child.kill().await.expect("killed");
    }
    let (status, body) = call(&cluster.nodes[front], "PUT", "/shards/second", &large()).await;
    assert_eq!(status, 200, "two down is within the parity: {body}");
    assert_eq!(
        count(&cluster.meta, "heals").await,
        1,
        "the degraded write is listed"
    );

    cluster.nodes[5].child.kill().await.expect("killed");
    let (status, body) = call(&cluster.nodes[front], "PUT", "/shards/third", &large()).await;
    assert_eq!(status, 503, "{body}");
    assert!(body.contains("ServiceUnavailable"), "{body}");
    assert_eq!(
        call(&cluster.nodes[front], "HEAD", "/shards/third", b"")
            .await
            .0,
        404,
        "nothing committed"
    );
}
