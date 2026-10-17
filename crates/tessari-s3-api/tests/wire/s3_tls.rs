//! The S3 API over a real local socket and TLS: a client that trusts the listener's certificate completes the
//! handshake and is answered by the S3 surface; a client speaking plain HTTP to the same port gets no HTTP answer.

use std::sync::Arc;

use rustls::pki_types::ServerName;
use tessari_s3_api::serve_s3;
use tessari_s3_infrastructure::tls::{ServerTls, listener_config};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::{NOW, state};

const REQUEST: &[u8] = b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";

/// The S3 surface over TLS with a fresh certificate for `localhost` on a free local port: its address, the task
/// serving it, and the certificate a client must trust.
async fn serving() -> (
    String,
    tokio::task::JoinHandle<()>,
    rustls::pki_types::CertificateDer<'static>,
) {
    let key = rcgen::KeyPair::generate().expect("a key");
    let certificate = rcgen::CertificateParams::new(vec!["localhost".to_owned()])
        .expect("parameters")
        .self_signed(&key)
        .expect("a certificate");
    let tls = ServerTls::new(
        certificate.pem().into_bytes(),
        key.serialize_pem().into_bytes(),
    );
    let config = listener_config(&tls, "TESSARIDB_S3_TLS_CERT", "TESSARIDB_S3_TLS_KEY")
        .expect("listener TLS");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a free port");
    let address = listener.local_addr().expect("bound").to_string();
    let server = tokio::spawn(async move {
        serve_s3(
            listener,
            state(NOW, 16),
            Some(config),
            std::future::pending(),
        )
        .await
        .expect("serves");
    });
    (address, server, certificate.der().clone())
}

async fn answer(stream: &mut (impl AsyncReadExt + AsyncWriteExt + Unpin)) -> String {
    stream.write_all(REQUEST).await.expect("request sent");
    let mut bytes = Vec::new();
    let _read = stream.read_to_end(&mut bytes).await;
    String::from_utf8_lossy(&bytes).into_owned()
}

#[tokio::test]
async fn the_s3_api_answers_over_tls_and_not_in_the_clear() {
    let (address, server, trusted) = serving().await;
    let mut roots = rustls::RootCertStore::empty();
    roots.add(trusted).expect("trusted");
    let client = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("versions")
    .with_root_certificates(roots)
    .with_no_client_auth();
    let tcp = tokio::net::TcpStream::connect(&address)
        .await
        .expect("connects");
    let mut tls = tokio_rustls::TlsConnector::from(Arc::new(client))
        .connect(ServerName::try_from("localhost").expect("name"), tcp)
        .await
        .expect("the handshake completes");
    let secure = answer(&mut tls).await;
    assert!(secure.starts_with("HTTP/1.1 403"), "{secure}");
    assert!(secure.contains("<Code>AccessDenied</Code>"), "{secure}");

    let mut plain = tokio::net::TcpStream::connect(&address)
        .await
        .expect("connects");
    let clear = answer(&mut plain).await;
    assert!(!clear.starts_with("HTTP/"), "{clear}");
    server.abort();
}
