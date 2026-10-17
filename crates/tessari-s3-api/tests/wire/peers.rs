//! One node calling another over a real local socket and mutual TLS: [`RemoteShards`] against the internal surface
//! — a shard stored, read whole and by block range, and removed; a caller whose certificate does not speak for it, a
//! caller from another authority, and a server that is not the node dialled each refused with nothing stored; a node
//! that is not admitted refused; a caller speaking plain HTTP answered nothing; a peer that does not answer reported as
//! unavailable within the call's budget.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use futures_util::stream::{self, TryStreamExt};
use tessari_s3_api::{ApiState, serve_internal};
use tessari_s3_infrastructure::InternalTls;
use tessari_s3_infrastructure::peer::PeerError;
use tessari_s3_infrastructure::tls::server_config;
use tessari_s3_storage::Error;
use tessari_s3_storage::peers::{PeerAt, RemoteShards, ShardRef, ShardShape};
use tessari_s3_types::NodeId;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub(crate) use crate::authority::Authority;
use crate::internal::{Admissions, internal_with};

const SHARD: ShardRef = ShardRef {
    id: [9; 16],
    index: 4,
};
/// 160 bytes in blocks of 64, plain.
const SHAPE: ShardShape = ShardShape {
    block_len: 64,
    size: 160,
    sealed: false,
};
const BUDGET: Duration = Duration::from_secs(10);

/// A node serving the internal surface over mutual TLS with `tls` on a free local port, asking `admissions`: its
/// address, the task serving it, and its data directory.
pub(crate) async fn serving_as(
    tls: &InternalTls,
    admissions: Arc<Admissions>,
) -> (String, tokio::task::JoinHandle<()>, std::path::PathBuf) {
    let (state, dir) = internal_with(ApiState::system_clock(), admissions);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a free port");
    let address = listener.local_addr().expect("bound").to_string();
    let config = server_config(tls).expect("server TLS");
    let server = tokio::spawn(async move {
        serve_internal(listener, state, Some(config), std::future::pending())
            .await
            .expect("serves");
    });
    (address, server, dir)
}

/// `n2` calling, with `tls`.
pub(crate) fn caller(tls: &InternalTls) -> RemoteShards {
    RemoteShards::new(NodeId::new("n2").expect("node id"), tls).expect("client")
}

fn n1(address: &str) -> PeerAt {
    PeerAt {
        node: NodeId::new("n1").expect("node id"),
        endpoint: address.to_owned(),
    }
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
    peer: &PeerAt,
    blocks: Option<std::ops::Range<u64>>,
) -> Vec<u8> {
    let parts: Vec<Bytes> = peers
        .get(peer, SHARD, 160, false, blocks, BUDGET)
        .await
        .expect("read")
        .try_collect()
        .await
        .expect("body");
    parts.concat()
}

#[tokio::test]
async fn a_node_stores_reads_and_removes_a_shard_on_another() {
    let authority = Authority::new();
    let (address, server, dir) = serving_as(&authority.issue("n1"), Arc::default()).await;
    let peers = caller(&authority.issue("n2"));
    let peer = n1(&address);
    let bytes = body();
    peers
        .put(&peer, SHARD, SHAPE, chunked(&bytes), BUDGET)
        .await
        .expect("stored");
    assert_eq!(read(&peers, &peer, None).await, bytes);
    assert_eq!(read(&peers, &peer, Some(1..3)).await, bytes[64..].to_vec());
    peers.delete(&peer, SHARD, BUDGET).await.expect("removed");
    assert_eq!(
        peers
            .get(&peer, SHARD, 160, false, None, BUDGET)
            .await
            .err(),
        Some(Error::PeerRefused { status: 404 })
    );
    server.abort();
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn a_caller_whose_certificate_is_not_its_own_or_not_the_clusters_stores_nothing() {
    let authority = Authority::new();
    let (address, server, dir) = serving_as(&authority.issue("n1"), Arc::default()).await;
    let peer = n1(&address);
    // Claims to be n2 with n3's certificate: the transport accepts it, the surface refuses it.
    let impostor = caller(&authority.issue("n3"));
    assert_eq!(
        impostor
            .put(&peer, SHARD, SHAPE, chunked(&body()), BUDGET)
            .await,
        Err(Error::PeerRefused { status: 401 })
    );
    // Issued by another authority: the handshake itself fails. The reason names the TLS stage, which varies.
    let stranger = caller(&Authority::new().issue("n2"));
    let refused = stranger
        .put(&peer, SHARD, SHAPE, chunked(&body()), BUDGET)
        .await;
    assert!(
        matches!(refused, Err(Error::Peer(PeerError::Unavailable { .. }))),
        "{refused:?}"
    );
    // Plain HTTP: the server reads a TLS handshake where the request is, and answers no request.
    let mut plain = tokio::net::TcpStream::connect(&address)
        .await
        .expect("connects");
    let request = format!(
        "PUT /internal/v1/shards/{}/4?block=64&size=0 HTTP/1.1\r\nhost: n1\r\nx-tessari-node: n2\r\ncontent-length: 0\r\n\r\n",
        "09".repeat(16)
    );
    plain.write_all(request.as_bytes()).await.expect("sent");
    let mut answer = Vec::new();
    let _closed = tokio::time::timeout(BUDGET, plain.read_to_end(&mut answer)).await;
    assert!(
        !answer.starts_with(b"HTTP/"),
        "a plain request was answered"
    );
    assert_eq!(
        caller(&authority.issue("n2"))
            .get(&peer, SHARD, 160, false, None, BUDGET)
            .await
            .err(),
        Some(Error::PeerRefused { status: 404 }),
        "nothing was stored"
    );
    server.abort();
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn a_server_that_is_not_the_node_dialled_is_not_reached() {
    let authority = Authority::new();
    // n4 answers at the address the caller believes is n1's.
    let (address, server, dir) = serving_as(&authority.issue("n4"), Arc::default()).await;
    let peers = caller(&authority.issue("n2"));
    let refused = peers
        .put(&n1(&address), SHARD, SHAPE, chunked(&body()), BUDGET)
        .await;
    assert!(
        matches!(refused, Err(Error::Peer(PeerError::Unavailable { .. }))),
        "{refused:?}"
    );
    let as_n4 = PeerAt {
        node: NodeId::new("n4").expect("node id"),
        endpoint: address.clone(),
    };
    assert_eq!(
        peers
            .get(&as_n4, SHARD, 160, false, None, BUDGET)
            .await
            .err(),
        Some(Error::PeerRefused { status: 404 }),
        "nothing was stored on the server that was not dialled"
    );
    server.abort();
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn a_node_that_is_not_admitted_is_refused() {
    let authority = Authority::new();
    let admissions = Arc::new(Admissions::default());
    admissions.answer(2);
    let (address, server, dir) = serving_as(&authority.issue("n1"), admissions).await;
    let peers = caller(&authority.issue("n2"));
    assert_eq!(
        peers.probe(&n1(&address), BUDGET).await,
        Err(Error::PeerRefused { status: 403 })
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
    let outcome = caller(&Authority::new().issue("n2"))
        .delete(&n1(&address), SHARD, Duration::from_millis(300))
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
        caller(&Authority::new().issue("n2"))
            .delete(&n1(&closed), SHARD, BUDGET)
            .await,
        Err(Error::Peer(PeerError::Unavailable {
            reason: "the connection could not be opened"
        }))
    );
}

#[tokio::test]
async fn a_probe_is_answered_by_an_admitted_member_and_bounded_by_its_budget() {
    let authority = Authority::new();
    let (address, server, dir) = serving_as(&authority.issue("n1"), Arc::default()).await;
    assert_eq!(
        caller(&authority.issue("n2"))
            .probe(&n1(&address), BUDGET)
            .await,
        Ok(())
    );
    assert_eq!(
        caller(&authority.issue("n3"))
            .probe(&n1(&address), BUDGET)
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
        caller(&authority.issue("n2"))
            .probe(&n1(&address), Duration::from_millis(300))
            .await,
        Err(Error::Peer(PeerError::Unavailable {
            reason: "no answer within the budget"
        }))
    );
    holder.abort();
}
