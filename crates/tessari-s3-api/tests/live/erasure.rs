//! A cluster member writing objects as RS(4, 2) shards: this node plus five peers serving their internal surface on
//! local ports, the metadata in a fresh TessariDB namespace. A large object lands as one shard on every node and is
//! reclaimed from every node after it is deleted; two nodes down still acknowledge and list the object for
//! healing; three nodes down acknowledge nothing and commit nothing.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tessari_s3_api::{ApiState, InternalState, serve_internal};
use tessari_s3_infrastructure::S3Config;
use tessari_s3_infrastructure::tessaridb::{Answer, MetaPool};
use tessari_s3_infrastructure::tls::server_config;
use tessari_s3_storage::Storage;
use tessari_s3_types::NodeId;

use crate::authority::Authority;
use crate::{ACCESS_KEY, IGNORED, SCHEMA_TURN, SECRET, call, need, scratch_dir, test_node};

const CLUSTER_SECRET: &str = "cluster-secret-0123456789abcdef0123";

/// A peer node serving shards from its own data directory.
pub(crate) struct Peer {
    pub(crate) address: String,
    pub(crate) server: tokio::task::JoinHandle<()>,
    pub(crate) dir: PathBuf,
}

/// This node's state and storage, the planter pool, its data directory and its five peers.
pub(crate) struct Member {
    pub(crate) state: ApiState,
    /// The member's console, over the same storage.
    pub(crate) console: tessari_s3_api::ConsoleState,
    pub(crate) storage: Storage,
    pub(crate) planter: MetaPool,
    pub(crate) dir: PathBuf,
    pub(crate) peers: Vec<Peer>,
    /// The member's configuration, for a test that starts more peers in its namespace.
    pub(crate) config: S3Config,
    /// The authority that issued every node's certificate, for a test that starts more peers.
    pub(crate) authority: Authority,
}

impl Drop for Member {
    fn drop(&mut self) {
        for peer in &self.peers {
            peer.server.abort();
            std::fs::remove_dir_all(&peer.dir).ok();
        }
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

/// `n1`'s TLS material from `authority`, written into `dir`: the certificate, key and authority files.
fn tls_files(authority: &Authority, dir: &Path) -> [String; 3] {
    let tls = authority.issue("n1");
    std::fs::create_dir_all(dir).expect("the data directory");
    let contents = [
        ("n1-cert.pem", tls.certificate_pem.as_slice()),
        ("n1-key.pem", tls.key_pem()),
        ("ca.pem", tls.authority_pem.as_slice()),
    ];
    contents.map(|(name, bytes)| {
        let path = dir.join(name);
        std::fs::write(&path, bytes).expect("written");
        path.display().to_string()
    })
}

fn configuration(namespace: &str, dir: &Path, tls: [String; 3]) -> S3Config {
    let [cert, key, ca] = tls;
    let vars = [
        ("TESSARIDB_S3_ROOT_ACCESS_KEY", ACCESS_KEY.to_owned()),
        ("TESSARIDB_S3_ROOT_SECRET_KEY", SECRET.to_owned()),
        ("TESSARIDB_S3_META_ADDRESS", test_node()),
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
        ("TESSARIDB_S3_RECLAIM_GRACE_SECS", "0".to_owned()),
        ("TESSARIDB_S3_ERASURE", "4+2".to_owned()),
        ("TESSARIDB_S3_NODE_ID", "n1".to_owned()),
        ("TESSARIDB_S3_INTERNAL_LISTEN", "127.0.0.1:0".to_owned()),
        ("TESSARIDB_S3_INTERNAL_ADVERTISE", "127.0.0.1:1".to_owned()),
        ("TESSARIDB_S3_CLUSTER_SECRET", CLUSTER_SECRET.to_owned()),
        ("TESSARIDB_S3_INTERNAL_TLS_CERT", cert),
        ("TESSARIDB_S3_INTERNAL_TLS_KEY", key),
        ("TESSARIDB_S3_INTERNAL_TLS_CA", ca),
        ("TESSARIDB_S3_CONSOLE_LISTEN", "127.0.0.1:9101".to_owned()),
        // The live suite's own root key for sealing user access-key secrets, as `live_config` sets it.
        ("TESSARIDB_S3_IAM_KEY", "7e".repeat(32)),
    ];
    S3Config::from_lookup(|key| vars.iter().find(|(k, _)| *k == key).map(|(_, v)| v.clone()))
        .expect("live configuration")
}

/// Peer `node` in `config`'s namespace, holding the certificate `authority` issued for it, admitting the members that
/// namespace approved.
pub(crate) async fn peer(config: &S3Config, authority: &Authority, node: &str) -> Peer {
    let dir = scratch_dir();
    let storage = Storage::new(
        MetaPool::new(config.meta.clone()).expect("pool"),
        dir.clone(),
    );
    let members = Arc::new(storage.members().clone());
    let state = InternalState::new(ApiState::system_clock(), storage, members, 64);
    let tls = server_config(&authority.issue(node)).expect("server TLS");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a free port");
    let address = listener.local_addr().expect("bound").to_string();
    let server = tokio::spawn(async move {
        serve_internal(listener, state, Some(tls), std::future::pending())
            .await
            .expect("serves");
    });
    Peer {
        address,
        server,
        dir,
    }
}

/// A cluster member `n1` with peers `n2`..`n6` registered, in a fresh namespace.
pub(crate) async fn member() -> Member {
    let namespace = crate::namespaces::name();
    let dir = scratch_dir();
    let authority = Authority::new();
    let config = configuration(&namespace, &dir, tls_files(&authority, &dir));
    crate::namespaces::removed_at_end(&config.meta);
    let storage = Storage::clustered(
        MetaPool::new(config.meta.clone()).expect("pool"),
        config.data_dir.clone(),
        config.cluster.as_ref().expect("clustered"),
    )
    .expect("clustered storage")
    .with_iam_key(config.iam_key.clone())
    .with_reclaim_grace(config.reclaim_grace_secs);
    {
        let _turn = SCHEMA_TURN.lock().await;
        storage
            .prepare()
            .await
            .expect("schema applies on the test node");
    }
    storage
        .cluster()
        .register(&NodeId::new("n1").expect("id"), "127.0.0.1:1", None)
        .await
        .expect("registered");
    admitted(&storage, "n1").await;
    let mut peers = Vec::new();
    for n in 2..=6 {
        let peer = peer(&config, &authority, &format!("n{n}")).await;
        let node = NodeId::new(&format!("n{n}")).expect("id");
        admitted(&storage, node.as_str()).await;
        storage
            .cluster()
            .register(&node, &peer.address, None)
            .await
            .expect("registered");
        peers.push(peer);
    }
    Member {
        state: ApiState::new(&config, ApiState::system_clock(), storage.clone()),
        console: tessari_s3_api::ConsoleState::new(
            &config,
            ApiState::system_clock(),
            storage.clone(),
        )
        .expect("built")
        .expect("a console is configured"),
        storage,
        planter: MetaPool::new(config.meta.clone()).expect("pool"),
        dir,
        peers,
        config,
        authority,
    }
}

/// `node` approved as a member of the cluster, presenting any certificate the authority issued for it.
pub(crate) async fn admitted(storage: &Storage, node: &str) {
    storage
        .members()
        .approve(
            &NodeId::new(node).expect("id"),
            None,
            "live-test",
            "a member of the test cluster",
        )
        .await
        .expect("approved");
}

/// Shard files under `dir`, temporary ones included.
pub(crate) fn shard_files(dir: &Path) -> usize {
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

/// Every node's shard file count, this node first.
fn spread(member: &Member) -> Vec<usize> {
    std::iter::once(&member.dir)
        .chain(member.peers.iter().map(|peer| &peer.dir))
        .map(|dir| shard_files(dir))
        .collect()
}

pub(crate) async fn count(pool: &MetaPool, table: &str) -> usize {
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

/// 300 KiB: inside one stripe.
fn large() -> Vec<u8> {
    (0..300 * 1024_u32)
        .map(|i| u8::try_from(i % 251).expect("small"))
        .collect()
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn an_object_is_written_as_one_shard_per_node_and_reclaimed_from_every_node() {
    let _why = IGNORED;
    let member = member().await;
    assert_eq!(
        call(&member.state, "PUT", "/shards", vec![], b"")
            .await
            .status,
        200
    );
    let put = call(&member.state, "PUT", "/shards/big", vec![], &large()).await;
    assert_eq!(put.status, 200, "{}", put.body);
    assert_eq!(spread(&member), vec![1; 6], "one shard on every node");
    assert_eq!(count(&member.planter, "shard_sets").await, 1);
    assert_eq!(
        count(&member.planter, "heal_queue").await,
        0,
        "every shard durable"
    );
    let usage = member.storage.usage();
    usage.refresh("n1").await.expect("measured");
    let measured = usage.latest().await.expect("read").expect("taken");
    assert_eq!(
        measured.buckets.first().map(|bucket| bucket.raw_bytes),
        Some(6 * 256 * 1024),
        "300 KiB is one stripe: six 256 KiB blocks under 4+2"
    );
    let read = call(&member.state, "GET", "/shards/big", vec![], b"").await;
    assert_eq!(read.status, 200, "{}", read.body);
    assert_eq!(read.bytes, large(), "read back from the shards");
    let deleted = call(&member.state, "DELETE", "/shards/big", vec![], b"").await;
    assert_eq!(deleted.status, 204);
    let reclaimed = member
        .storage
        .objects()
        .reclaim(0, 100)
        .await
        .expect("reclaimed");
    assert_eq!(reclaimed.removed, 1);
    assert_eq!(spread(&member), vec![0; 6], "no shard left on any node");
    assert_eq!(count(&member.planter, "shard_sets").await, 0);
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn nodes_down_within_the_parity_still_acknowledge_and_list_the_object_for_healing() {
    let _why = IGNORED;
    let member = member().await;
    assert_eq!(
        call(&member.state, "PUT", "/shards", vec![], b"")
            .await
            .status,
        200
    );
    member.peers[1].server.abort();
    member.peers[3].server.abort();
    let put = call(&member.state, "PUT", "/shards/big", vec![], &large()).await;
    assert_eq!(put.status, 200, "{}", put.body);
    assert_eq!(
        spread(&member).iter().sum::<usize>(),
        4,
        "{:?}",
        spread(&member)
    );
    assert_eq!(count(&member.planter, "heal_queue").await, 1);
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn below_the_write_quorum_nothing_is_acknowledged_or_committed() {
    let _why = IGNORED;
    let member = member().await;
    assert_eq!(
        call(&member.state, "PUT", "/shards", vec![], b"")
            .await
            .status,
        200
    );
    for down in [0, 2, 4] {
        member.peers[down].server.abort();
    }
    let put = call(&member.state, "PUT", "/shards/big", vec![], &large()).await;
    assert_eq!(
        (put.status, put.code.as_deref()),
        (503, Some("ServiceUnavailable")),
        "{}",
        put.body
    );
    let head = call(&member.state, "HEAD", "/shards/big", vec![], b"").await;
    assert_eq!(head.status, 404, "nothing committed");
    assert_eq!(
        count(&member.planter, "reclaims").await,
        1,
        "the data id stays queued"
    );
}
