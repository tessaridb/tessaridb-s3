//! One node calling another over a real local socket: [`RemoteShards`] against the internal router — a shard stored,
//! read whole and by block range, and removed; a caller without the cluster secret refused; and a peer that does not
//! answer reported as unavailable within the call's budget; a probe answered by the cluster only.

use std::time::Duration;

use axum::body::Bytes;
use futures_util::stream::{self, TryStreamExt};
use tessari_s3_api::{ApiState, internal_router};
use tessari_s3_infrastructure::peer::PeerError;
use tessari_s3_storage::Error;
use tessari_s3_storage::peers::{RemoteShards, ShardRef};
use tessari_s3_types::{NodeId, SecretKey};

use crate::internal::{CLUSTER_SECRET, internal_with};

const SHARD: ShardRef = ShardRef {
    id: [9; 16],
    index: 4,
};
const BUDGET: Duration = Duration::from_secs(10);

/// A node serving the internal routes on a free local port: its address and the task serving it.
pub(crate) async fn serving() -> (String, tokio::task::JoinHandle<()>, std::path::PathBuf) {
    let (state, dir) = internal_with(ApiState::system_clock());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a free port");
    let address = listener.local_addr().expect("bound").to_string();
    let server = tokio::spawn(async move {
        axum::serve(listener, internal_router(state))
            .await
            .expect("serves");
    });
    (address, server, dir)
}

pub(crate) fn caller(secret: &str) -> RemoteShards {
    RemoteShards::new(
        NodeId::new("n2").expect("node id"),
        SecretKey::new(secret.to_owned()),
        None,
    )
    .expect("client")
}

/// 2.5 blocks of 64 bytes, sent in uneven chunks.
fn body() -> Vec<u8> {
    (0..160_u32)
        .map(|i| u8::try_from(i % 251).expect("small"))
        .collect()
}

fn chunked(bytes: &[u8]) -> impl futures_util::Stream<Item = std::io::Result<Bytes>> + use<> {
    let chunks: Vec<std::io::Result<Bytes>> = bytes
        .chunks(37)
        .map(|chunk| Ok(Bytes::copy_from_slice(chunk)))
        .collect();
    stream::iter(chunks)
}

async fn read(
    peers: &RemoteShards,
    address: &str,
    blocks: Option<std::ops::Range<u64>>,
) -> Vec<u8> {
    let parts: Vec<Bytes> = peers
        .get(address, SHARD, 160, blocks, BUDGET)
        .await
        .expect("read")
        .try_collect()
        .await
        .expect("body");
    parts.concat()
}

#[tokio::test]
async fn a_node_stores_reads_and_removes_a_shard_on_another() {
    let (address, server, dir) = serving().await;
    let peers = caller(CLUSTER_SECRET);
    let bytes = body();
    peers
        .put(&address, SHARD, 64, 160, chunked(&bytes), BUDGET)
        .await
        .expect("stored");
    assert_eq!(read(&peers, &address, None).await, bytes);
    assert_eq!(
        read(&peers, &address, Some(1..3)).await,
        bytes[64..].to_vec()
    );
    peers
        .delete(&address, SHARD, BUDGET)
        .await
        .expect("removed");
    assert_eq!(
        peers.get(&address, SHARD, 160, None, BUDGET).await.err(),
        Some(Error::PeerRefused { status: 404 })
    );
    server.abort();
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn a_caller_without_the_cluster_secret_stores_nothing() {
    let (address, server, dir) = serving().await;
    let stranger = caller("another-secret-0123456789abcdef0123");
    assert_eq!(
        stranger
            .put(&address, SHARD, 64, 160, chunked(&body()), BUDGET)
            .await,
        Err(Error::PeerRefused { status: 401 })
    );
    assert_eq!(
        caller(CLUSTER_SECRET)
            .get(&address, SHARD, 160, None, BUDGET)
            .await
            .err(),
        Some(Error::PeerRefused { status: 404 })
    );
    server.abort();
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn a_peer_that_does_not_answer_is_unavailable_within_the_budget() {
    // Accepts connections and never answers.
    let silent = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a free port");
    let address = silent.local_addr().expect("bound").to_string();
    let holder = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = silent.accept().await {
            held.push(socket);
        }
    });
    let started = std::time::Instant::now();
    let outcome = caller(CLUSTER_SECRET)
        .delete(&address, SHARD, Duration::from_millis(300))
        .await;
    assert_eq!(
        outcome,
        Err(Error::Peer(PeerError::Unavailable {
            reason: "no answer within the budget"
        }))
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "bounded by the budget"
    );
    holder.abort();
    // A port nobody listens on any more.
    let closed = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port");
        listener.local_addr().expect("bound").to_string()
    };
    assert_eq!(
        caller(CLUSTER_SECRET).delete(&closed, SHARD, BUDGET).await,
        Err(Error::Peer(PeerError::Unavailable {
            reason: "the connection could not be opened"
        }))
    );
}

#[tokio::test]
async fn a_probe_is_answered_by_the_cluster_only_and_bounded_by_its_budget() {
    let (address, server, dir) = serving().await;
    assert_eq!(caller(CLUSTER_SECRET).probe(&address, BUDGET).await, Ok(()));
    assert_eq!(
        caller("another-secret-0123456789abcdef0123")
            .probe(&address, BUDGET)
            .await,
        Err(Error::PeerRefused { status: 401 })
    );
    server.abort();
    std::fs::remove_dir_all(dir).ok();
    let silent = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a free port");
    let address = silent.local_addr().expect("bound").to_string();
    let holder = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = silent.accept().await {
            held.push(socket);
        }
    });
    assert_eq!(
        caller(CLUSTER_SECRET)
            .probe(&address, Duration::from_millis(300))
            .await,
        Err(Error::Peer(PeerError::Unavailable {
            reason: "no answer within the budget"
        }))
    );
    holder.abort();
}
