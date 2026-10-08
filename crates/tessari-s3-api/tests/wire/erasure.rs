//! Erasure-coded writes over a real cluster of local internal surfaces: an object spread over k + m nodes, rebuilt
//! from any k shards read back from them; nodes down within the parity still acknowledged; below the write quorum
//! never acknowledged; a node that stops taking blocks given up within the stall time.

use std::time::Duration;

use axum::body::Bytes;
use futures_util::TryStreamExt;
use tessari_s3_core::erasure::{Code, Stripes};
use tessari_s3_core::objects::checksum::Hashes;
use tessari_s3_storage::Error;
use tessari_s3_storage::erasure::{Destination, ErasurePlan, ErasureUpload, ErasureUploaded};
use tessari_s3_storage::peers::{RemoteShards, ShardRef};
use tessari_s3_storage::shards::ShardService;

use crate::internal::{CLUSTER_SECRET, storage_at};
use crate::peers::{caller, serving};

const ID: [u8; 16] = [5; 16];
const STRIPE: u32 = 4096;
const STALL: Duration = Duration::from_millis(200);

/// What one test's cluster is made of, removed when it is dropped.
struct Cluster {
    local: ShardService,
    peers: RemoteShards,
    destinations: Vec<Destination>,
    servers: Vec<tokio::task::JoinHandle<()>>,
    dirs: Vec<std::path::PathBuf>,
}

impl Drop for Cluster {
    fn drop(&mut self) {
        for server in &self.servers {
            server.abort();
        }
        for dir in &self.dirs {
            std::fs::remove_dir_all(dir).ok();
        }
    }
}

/// A port nobody listens on.
fn closed_port() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port");
    listener.local_addr().expect("bound").to_string()
}

/// This node for shard 0 and a serving peer for every other shard, except the shard indexes in `down`, which point
/// at closed ports.
async fn cluster(width: usize, down: &[usize]) -> Cluster {
    let local_dir =
        std::env::temp_dir().join(format!("tessari-s3-data-{}", uuid::Uuid::new_v4().simple()));
    let mut cluster = Cluster {
        local: storage_at(local_dir.clone()).shards().clone(),
        peers: caller(CLUSTER_SECRET),
        destinations: vec![Destination::Local],
        servers: Vec::new(),
        dirs: vec![local_dir],
    };
    for index in 1..width {
        if down.contains(&index) {
            cluster
                .destinations
                .push(Destination::Remote(closed_port()));
        } else {
            let (address, server, dir) = serving().await;
            cluster.destinations.push(Destination::Remote(address));
            cluster.servers.push(server);
            cluster.dirs.push(dir);
        }
    }
    cluster
}

/// 3.5 stripes of bytes that differ from stripe to stripe.
fn body() -> Vec<u8> {
    (0..14_336_u32)
        .map(|i| u8::try_from(i.wrapping_mul(7).wrapping_add(i >> 12) % 251).expect("small"))
        .collect()
}

/// Writes `bytes` under `code` in uneven pieces; the first refusal, from an append or the finish, is the answer.
async fn write(cluster: &Cluster, code: Code, bytes: &[u8]) -> Result<ErasureUploaded, Error> {
    let plan = ErasurePlan {
        id: ID,
        code,
        stripe_size: STRIPE,
        size: u64::try_from(bytes.len()).expect("small"),
        destinations: cluster.destinations.clone(),
        stall: STALL,
    };
    let mut upload = ErasureUpload::start(plan, &cluster.local, &cluster.peers)?;
    for piece in bytes.chunks(1000) {
        upload.append(piece).await?;
    }
    upload.finish().await
}

fn code(data: u8, parity: u8) -> Code {
    Code::new(data, parity).expect("code")
}

/// Shard `index` as stored on its node.
async fn shard(cluster: &Cluster, index: usize, size: u64) -> Vec<u8> {
    let shard = ShardRef {
        id: ID,
        index: u16::try_from(index).expect("small"),
    };
    match &cluster.destinations[index] {
        Destination::Local => {
            let reader = cluster
                .local
                .open(shard.id, shard.index, size)
                .await
                .expect("open");
            let mut bytes = Vec::new();
            for block in 0..size.div_ceil(u64::from(reader.block_size())) {
                bytes.extend(reader.read_block(block).await.expect("block"));
            }
            bytes
        }
        Destination::Remote(address) => {
            let parts: Vec<Bytes> = cluster
                .peers
                .get(address, shard, size, None, Duration::from_secs(10))
                .await
                .expect("read")
                .try_collect()
                .await
                .expect("body");
            parts.concat()
        }
    }
}

/// The object rebuilt stripe by stripe from the shards given.
fn rebuilt(code: Code, shards: &[Option<Vec<u8>>], len: usize) -> Vec<u8> {
    let stripes = Stripes::new(code, STRIPE).expect("stripes");
    let block = stripes.block_len();
    let stripe = usize::try_from(STRIPE).expect("small");
    let mut object = Vec::new();
    for (index, wanted) in vec![0_u8; len].chunks(stripe).map(<[u8]>::len).enumerate() {
        let mut blocks: Vec<Option<Vec<u8>>> = shards
            .iter()
            .map(|shard| {
                shard
                    .as_ref()
                    .and_then(|bytes| bytes.chunks(block).nth(index))
                    .map(<[u8]>::to_vec)
            })
            .collect();
        object.extend(stripes.decode(&mut blocks, wanted).expect("decodes"));
    }
    object
}

#[tokio::test]
async fn an_object_is_spread_over_its_nodes_and_any_k_shards_rebuild_it() {
    let cluster = cluster(6, &[]).await;
    let bytes = body();
    let written = write(&cluster, code(4, 2), &bytes)
        .await
        .expect("acknowledged");
    assert_eq!(written.durable, vec![true; 6]);
    let mut expected = Hashes::new();
    expected.update(&bytes);
    assert_eq!(
        written.digests,
        expected.finish(),
        "digests of the whole object"
    );
    // 4 stripes of 1024-byte blocks per shard.
    let mut shards = Vec::new();
    for index in 0..6 {
        let stored = shard(&cluster, index, 4096).await;
        assert_eq!(stored.len(), 4096, "shard {index}");
        shards.push(Some(stored));
    }
    assert_eq!(
        rebuilt(code(4, 2), &shards, bytes.len()),
        bytes,
        "all shards"
    );
    shards[1] = None;
    shards[4] = None;
    assert_eq!(rebuilt(code(4, 2), &shards, bytes.len()), bytes, "any four");
}

#[tokio::test]
async fn nodes_down_within_the_parity_are_written_around() {
    let cluster = cluster(6, &[2, 3]).await;
    let written = write(&cluster, code(4, 2), &body())
        .await
        .expect("acknowledged on four of six");
    assert_eq!(written.durable, vec![true, true, false, false, true, true]);
}

#[tokio::test]
async fn below_the_write_quorum_nothing_is_acknowledged() {
    let three_down = cluster(6, &[1, 3, 5]).await;
    assert_eq!(
        write(&three_down, code(4, 2), &body()).await,
        Err(Error::Quorum {
            durable: 3,
            needed: 4
        })
    );
    // k = m: half the shards could belong to either of two writers, so the quorum is k + 1.
    let half = cluster(4, &[1]).await;
    assert_eq!(
        write(&half, code(2, 2), &body())
            .await
            .map(|written| written.durable),
        Ok(vec![true, false, true, true])
    );
    let short = cluster(4, &[1, 2]).await;
    assert_eq!(
        write(&short, code(2, 2), &body()).await,
        Err(Error::Quorum {
            durable: 2,
            needed: 3
        })
    );
}

#[tokio::test]
async fn a_node_that_stops_answering_is_given_up_within_its_budget() {
    let mut cluster = cluster(6, &[]).await;
    // Shard 5's node accepts connections and never answers.
    let silent = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a free port");
    cluster.destinations[5] = Destination::Remote(silent.local_addr().expect("bound").to_string());
    let holder = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = silent.accept().await {
            held.push(socket);
        }
    });
    let started = std::time::Instant::now();
    let written = write(&cluster, code(4, 2), &body())
        .await
        .expect("acknowledged");
    assert_eq!(written.durable, vec![true, true, true, true, true, false]);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    holder.abort();
}

/// A peer that takes every byte it is sent and then refuses, so its failure is known only once the body is through.
async fn refusing_peer() -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a free port");
    let address = listener.local_addr().expect("bound").to_string();
    let app = axum::Router::new().fallback(|body: axum::body::Body| async move {
        let _drained = axum::body::to_bytes(body, 1 << 20).await;
        axum::http::StatusCode::SERVICE_UNAVAILABLE
    });
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serves");
    });
    (address, server)
}

#[tokio::test]
async fn shards_refused_after_their_last_block_still_count_against_the_quorum() {
    let mut cluster = cluster(6, &[]).await;
    for index in [1, 3, 5] {
        let (address, server) = refusing_peer().await;
        cluster.destinations[index] = Destination::Remote(address);
        cluster.servers.push(server);
    }
    assert_eq!(
        write(&cluster, code(4, 2), &body()).await,
        Err(Error::Quorum {
            durable: 3,
            needed: 4
        })
    );
}
