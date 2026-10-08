//! The internal surface over mutual TLS: a shard crosses between two nodes whose certificates one cluster authority
//! issued; a caller holding another authority's certificate, and one speaking plain HTTP, are refused before any
//! request is read.

use std::time::Duration;

use axum::body::Bytes;
use futures_util::stream::{self, TryStreamExt};
use tessari_s3_api::{ApiState, serve_internal};
use tessari_s3_infrastructure::InternalTls;
use tessari_s3_infrastructure::tls::server_config;
use tessari_s3_storage::Error;
use tessari_s3_storage::peers::{RemoteShards, ShardRef};
use tessari_s3_types::{NodeId, SecretKey};

use crate::internal::{CLUSTER_SECRET, internal_with};

const SHARD: ShardRef = ShardRef {
    id: [7; 16],
    index: 1,
};
const BUDGET: Duration = Duration::from_secs(10);

/// A certificate authority minted for one test, and the node certificates it issues for `127.0.0.1`.
struct Authority {
    certificate: rcgen::Certificate,
    key: rcgen::KeyPair,
}

impl Authority {
    fn new() -> Self {
        let mut params = rcgen::CertificateParams::new(Vec::new()).expect("authority parameters");
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let key = rcgen::KeyPair::generate().expect("authority key");
        let certificate = params.self_signed(&key).expect("authority certificate");
        Self { certificate, key }
    }

    /// A node's TLS material: a certificate for the host it advertises, its key, and this authority.
    fn node(&self) -> InternalTls {
        let params =
            rcgen::CertificateParams::new(vec!["127.0.0.1".to_owned()]).expect("node parameters");
        let key = rcgen::KeyPair::generate().expect("node key");
        let certificate = params
            .signed_by(&key, &self.certificate, &self.key)
            .expect("node certificate");
        InternalTls::new(
            certificate.pem().into_bytes(),
            key.serialize_pem().into_bytes(),
            self.certificate.pem().into_bytes(),
        )
    }
}

/// A node serving the internal routes over TLS on a free local port: its address and the task serving it.
async fn serving_tls(
    tls: &InternalTls,
) -> (String, tokio::task::JoinHandle<()>, std::path::PathBuf) {
    let (state, dir) = internal_with(ApiState::system_clock());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a free port");
    let address = listener.local_addr().expect("bound").to_string();
    let config = server_config(tls).expect("server configuration");
    let server = tokio::spawn(async move {
        serve_internal(listener, state, Some(config), std::future::pending())
            .await
            .expect("serves");
    });
    (address, server, dir)
}

fn caller(tls: Option<&InternalTls>) -> RemoteShards {
    RemoteShards::new(
        NodeId::new("n2").expect("node id"),
        SecretKey::new(CLUSTER_SECRET.to_owned()),
        tls,
    )
    .expect("client")
}

fn body() -> Vec<u8> {
    (0..160_u32)
        .map(|i| u8::try_from(i % 251).expect("small"))
        .collect()
}

fn once(bytes: Vec<u8>) -> impl futures_util::Stream<Item = std::io::Result<Bytes>> + use<> {
    stream::iter([Ok(Bytes::from(bytes))])
}

#[tokio::test]
async fn a_shard_crosses_mutual_tls_between_nodes_of_one_authority() {
    let authority = Authority::new();
    let (address, server, dir) = serving_tls(&authority.node()).await;
    let client = caller(Some(&authority.node()));
    let size = u64::try_from(body().len()).expect("small");
    client
        .put(&address, SHARD, 64, size, once(body()), BUDGET)
        .await
        .expect("stored over TLS");
    let read: Vec<Bytes> = client
        .get(&address, SHARD, size, None, BUDGET)
        .await
        .expect("read over TLS")
        .try_collect()
        .await
        .expect("the whole shard");
    assert_eq!(read.concat(), body());
    server.abort();
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn a_caller_of_another_authority_or_in_the_clear_is_refused_before_any_request() {
    let authority = Authority::new();
    let (address, server, dir) = serving_tls(&authority.node()).await;
    let size = u64::try_from(body().len()).expect("small");
    // Trusts this cluster's authority, so only the server's check of the client certificate can refuse it.
    let other = Authority::new().node();
    let stranger = InternalTls::new(
        other.certificate_pem.clone(),
        other.key_pem().to_vec(),
        authority.certificate.pem().into_bytes(),
    );
    for (who, client) in [
        ("another authority's certificate", caller(Some(&stranger))),
        ("plain HTTP", caller(None)),
    ] {
        let refused = client
            .put(&address, SHARD, 64, size, once(body()), BUDGET)
            .await;
        assert!(matches!(refused, Err(Error::Peer(_))), "{who}: {refused:?}");
    }
    assert!(!dir.join("s3data").exists(), "nothing reached the drive");
    server.abort();
    std::fs::remove_dir_all(dir).ok();
}
