//! A cluster member writing objects as RS(4, 2) shards: this node plus five peers serving their internal surface on
//! local ports, the metadata in a fresh TessariDB namespace. A large object lands as one shard on every node and is
//! reclaimed from every node after it is deleted; two nodes down still acknowledge and list the object for
//! healing; three nodes down acknowledge nothing and commit nothing.

use std::path::{Path, PathBuf};

use tessari_s3_api::{ApiState, InternalState, internal_router};
use tessari_s3_infrastructure::S3Config;
use tessari_s3_infrastructure::tessaridb::{Answer, MetaPool};
use tessari_s3_storage::Storage;
use tessari_s3_types::{NodeId, SecretKey};

use crate::{ACCESS_KEY, IGNORED, SCHEMA_TURN, SECRET, call, need, scratch_dir, test_node};

const CLUSTER_SECRET: &str = "cluster-secret-0123456789abcdef0123";

/// A peer node serving shards from its own data directory.
pub(crate) struct Peer {
    address: String,
    pub(crate) server: tokio::task::JoinHandle<()>,
    pub(crate) dir: PathBuf,
}

/// This node's state and storage, the planter pool, its data directory and its five peers.
pub(crate) struct Member {
    pub(crate) state: ApiState,
    storage: Storage,
    pub(crate) planter: MetaPool,
    pub(crate) dir: PathBuf,
    pub(crate) peers: Vec<Peer>,
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

fn configuration(namespace: &str, dir: &Path) -> S3Config {
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
        ("TESSARIDB_S3_ERASURE", "4+2".to_owned()),
        ("TESSARIDB_S3_NODE_ID", "n1".to_owned()),
        ("TESSARIDB_S3_INTERNAL_LISTEN", "127.0.0.1:0".to_owned()),
        ("TESSARIDB_S3_INTERNAL_ADVERTISE", "127.0.0.1:1".to_owned()),
        ("TESSARIDB_S3_CLUSTER_SECRET", CLUSTER_SECRET.to_owned()),
    ];
    S3Config::from_lookup(|key| vars.iter().find(|(k, _)| *k == key).map(|(_, v)| v.clone()))
        .expect("live configuration")
}

async fn peer(config: &S3Config) -> Peer {
    let dir = scratch_dir();
    let storage = Storage::new(
        MetaPool::new(config.meta.clone()).expect("pool"),
        Some(dir.clone()),
    );
    let state = InternalState::new(
        SecretKey::new(CLUSTER_SECRET.to_owned()),
        ApiState::system_clock(),
        storage,
        64,
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a free port");
    let address = listener.local_addr().expect("bound").to_string();
    let server = tokio::spawn(async move {
        axum::serve(listener, internal_router(state))
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
    let namespace = format!("t_{}", uuid::Uuid::new_v4().simple());
    let dir = scratch_dir();
    let config = configuration(&namespace, &dir);
    let storage = Storage::clustered(
        MetaPool::new(config.meta.clone()).expect("pool"),
        config.data_dir.clone(),
        config.cluster.as_ref().expect("clustered"),
    )
    .expect("clustered storage");
    {
        let _turn = SCHEMA_TURN.lock().await;
        storage
            .prepare()
            .await
            .expect("schema applies on the test node");
    }
    storage
        .cluster()
        .register(&NodeId::new("n1").expect("id"), "127.0.0.1:1")
        .await
        .expect("registered");
    let mut peers = Vec::new();
    for n in 2..=6 {
        let peer = peer(&config).await;
        let node = NodeId::new(&format!("n{n}")).expect("id");
        storage
            .cluster()
            .register(&node, &peer.address)
            .await
            .expect("registered");
        peers.push(peer);
    }
    Member {
        state: ApiState::new(&config, ApiState::system_clock(), storage.clone()),
        storage,
        planter: MetaPool::new(config.meta.clone()).expect("pool"),
        dir,
        peers,
    }
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

/// 300 KiB: past the inline size, inside one stripe.
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
        count(&member.planter, "heals").await,
        0,
        "every shard durable"
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
    assert_eq!(count(&member.planter, "heals").await, 1);
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
        count(&member.planter, "gc").await,
        1,
        "the data id stays queued"
    );
}
