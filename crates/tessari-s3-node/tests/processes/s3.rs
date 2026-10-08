//! Six `tessaridb-s3` processes as one RS(4, 2) cluster over a fresh TessariDB namespace, each with its own ports,
//! data directory and log.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use tessari_s3_infrastructure::S3Config;
use tessari_s3_infrastructure::tessaridb::{Answer, MetaPool, MetaSettings};
use tokio::process::{Child, Command};

pub const ACCESS_KEY: &str = "AKPROCESSTEST0000001";
pub const SECRET: &str = "process-test-secret-0123456789abcdef";
const CLUSTER_SECRET: &str = "process-cluster-secret-0123456789abcd";
/// How long one process may take to open its S3 port: schema, registration and both listeners.
const READY_WITHIN: Duration = Duration::from_secs(60);

/// Where the six processes keep their metadata, and as whom.
#[derive(Clone)]
pub struct Meta {
    /// Wire addresses, comma-separated.
    pub addresses: String,
    pub user: String,
    pub password: String,
    /// `TESSARIDB_S3_META_REPLICATION`, required for more than one address.
    pub replication: Option<String>,
    /// A directory holding `cert.pem`, `key.pem` and `ca.pem` for the internal surface; `None` keeps it plain.
    pub internal_tls: Option<PathBuf>,
}

/// A certificate authority minted for one cluster and the certificate it issues for `127.0.0.1`, which every local
/// node presents: written to a fresh directory the cluster removes when it is dropped.
pub fn internal_tls() -> PathBuf {
    let dir = scratch("tls");
    std::fs::create_dir_all(&dir).expect("tls directory");
    let mut authority = rcgen::CertificateParams::new(Vec::new()).expect("authority parameters");
    authority.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let authority_key = rcgen::KeyPair::generate().expect("authority key");
    let authority = authority.self_signed(&authority_key).expect("authority");
    let node_key = rcgen::KeyPair::generate().expect("node key");
    let node = rcgen::CertificateParams::new(vec!["127.0.0.1".to_owned()])
        .expect("node parameters")
        .signed_by(&node_key, &authority, &authority_key)
        .expect("node certificate");
    std::fs::write(dir.join("ca.pem"), authority.pem()).expect("ca written");
    std::fs::write(dir.join("cert.pem"), node.pem()).expect("cert written");
    std::fs::write(dir.join("key.pem"), node_key.serialize_pem()).expect("key written");
    dir
}

/// A free local port, released for the child to bind.
pub fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port");
    listener.local_addr().expect("bound").port()
}

/// A fresh directory under the temporary directory, named for this test suite.
pub fn scratch(kind: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "tessari-s3-{kind}-{}",
        uuid::Uuid::new_v4().simple()
    ))
}

/// One node: its process, its name, its addresses and its data directory.
pub struct Node {
    pub child: Child,
    pub name: String,
    pub s3: String,
    pub internal: String,
    pub dir: PathBuf,
}

/// The six nodes, the namespace they share and a pool into it; every process is killed and every directory removed
/// when it is dropped.
pub struct Cluster {
    pub namespace: String,
    pub nodes: Vec<Node>,
    pub meta: MetaPool,
    pub logs: PathBuf,
    settings: Meta,
}

impl Drop for Cluster {
    fn drop(&mut self) {
        for node in &mut self.nodes {
            let _gone = node.child.start_kill();
            std::fs::remove_dir_all(&node.dir).ok();
        }
        // A failing test names the logs in its message; they stay for the reader.
        if let Some(dir) = &self.settings.internal_tls {
            std::fs::remove_dir_all(dir).ok();
        }
        if !std::thread::panicking() {
            std::fs::remove_dir_all(&self.logs).ok();
        }
    }
}

/// The variables every node shares, plus this node's own.
fn environment(
    meta: &Meta,
    namespace: &str,
    node: &str,
    s3: &str,
    internal: &str,
    dir: &Path,
) -> Vec<(String, String)> {
    let mut vars: Vec<(String, String)> = [
        ("TESSARIDB_S3_LISTEN", s3),
        ("TESSARIDB_S3_ROOT_ACCESS_KEY", ACCESS_KEY),
        ("TESSARIDB_S3_ROOT_SECRET_KEY", SECRET),
        ("TESSARIDB_S3_META_ADDRESS", &meta.addresses),
        ("TESSARIDB_S3_META_USER", &meta.user),
        ("TESSARIDB_S3_META_PASSWORD", &meta.password),
        ("TESSARIDB_S3_META_NAMESPACE", namespace),
        ("TESSARIDB_S3_ERASURE", "4+2"),
        ("TESSARIDB_S3_NODE_ID", node),
        ("TESSARIDB_S3_INTERNAL_LISTEN", internal),
        ("TESSARIDB_S3_INTERNAL_ADVERTISE", internal),
        ("TESSARIDB_S3_CLUSTER_SECRET", CLUSTER_SECRET),
        ("TESSARIDB_S3_HEAL_INTERVAL_SECS", "1"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect();
    vars.push((
        "TESSARIDB_S3_DATA_DIR".to_owned(),
        dir.display().to_string(),
    ));
    if let Some(dir) = &meta.internal_tls {
        for (key, file) in [
            ("TESSARIDB_S3_INTERNAL_TLS_CERT", "cert.pem"),
            ("TESSARIDB_S3_INTERNAL_TLS_KEY", "key.pem"),
            ("TESSARIDB_S3_INTERNAL_TLS_CA", "ca.pem"),
        ] {
            vars.push((key.to_owned(), dir.join(file).display().to_string()));
        }
    }
    if let Some(replication) = &meta.replication {
        vars.push((
            "TESSARIDB_S3_META_REPLICATION".to_owned(),
            replication.clone(),
        ));
    }
    vars
}

/// The metadata settings a node of this cluster runs with.
pub fn settings(meta: &Meta, namespace: &str) -> MetaSettings {
    let vars = environment(
        meta,
        namespace,
        "n1",
        "127.0.0.1:1",
        "127.0.0.1:1",
        Path::new("/"),
    );
    S3Config::from_lookup(|key| {
        vars.iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.clone())
    })
    .expect("configuration")
    .meta
}

/// Starts node `name` at these addresses over `dir` and waits until its S3 port accepts, which the binary opens
/// only once it is registered.
async fn launch(
    meta: &Meta,
    namespace: &str,
    name: &str,
    logs: &Path,
    (s3, internal): (String, String),
    dir: PathBuf,
) -> Node {
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(logs.join(format!("{name}.log")))
        .expect("log file");
    let child = Command::new(env!("CARGO_BIN_EXE_tessaridb-s3"))
        .env_clear()
        .envs(environment(meta, namespace, name, &s3, &internal, &dir))
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
    Node {
        child,
        name: name.to_owned(),
        s3,
        internal,
        dir,
    }
}

impl Cluster {
    /// Six nodes over a fresh namespace of `meta`, started one after another: every node applies the schema, and
    /// concurrent applies conflict on the catalog (Q-S3-2).
    pub async fn start(meta: Meta) -> Self {
        let namespace = format!("t_{}", uuid::Uuid::new_v4().simple());
        let logs = scratch("logs");
        std::fs::create_dir_all(&logs).expect("log directory");
        let mut nodes = Vec::new();
        for n in 1..=6 {
            let addresses = (
                format!("127.0.0.1:{}", free_port()),
                format!("127.0.0.1:{}", free_port()),
            );
            let name = format!("n{n}");
            nodes.push(launch(&meta, &namespace, &name, &logs, addresses, scratch("data")).await);
        }
        Self {
            meta: MetaPool::new(settings(&meta, &namespace)).expect("pool"),
            namespace,
            nodes,
            logs,
            settings: meta,
        }
    }

    /// Starts node `index` again as it was: its name, its addresses and its data directory.
    pub async fn restart(&mut self, index: usize) {
        let old = &self.nodes[index];
        let (name, addresses, dir) = (
            old.name.clone(),
            (old.s3.clone(), old.internal.clone()),
            old.dir.clone(),
        );
        self.nodes[index] = launch(
            &self.settings,
            &self.namespace,
            &name,
            &self.logs,
            addresses,
            dir,
        )
        .await;
    }
}

/// Shard files under `dir`.
pub fn shard_files(dir: &Path) -> usize {
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

pub async fn count(pool: &MetaPool, table: &str) -> usize {
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
