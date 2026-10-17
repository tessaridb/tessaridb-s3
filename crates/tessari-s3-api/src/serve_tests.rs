use std::sync::Arc;

use axum::Extension;
use axum::routing::get;
use rustls::pki_types::ServerName;
use tessari_s3_infrastructure::tls::{ServerTls, listener_config};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::{OverTls, serve};

const REQUEST: &[u8] = b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";

/// A router answering whether the request it served was marked as having arrived over TLS.
fn echo() -> axum::Router {
    axum::Router::new().route(
        "/",
        get(|over: Option<Extension<OverTls>>| async move {
            if over.is_some() {
                "over-tls"
            } else {
                "in-clear"
            }
        }),
    )
}

/// `echo` served on a free local port, over TLS for `localhost` when `secure`: its address, the task serving it,
/// and the certificate a client must trust.
async fn serving(
    secure: bool,
) -> (
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
    let config = listener_config(&tls, "CERT", "KEY").expect("listener TLS");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a free port");
    let address = listener.local_addr().expect("bound").to_string();
    let server = tokio::spawn(async move {
        serve(
            listener,
            echo(),
            secure.then_some(config),
            std::future::pending(),
            "test",
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
async fn a_request_is_marked_as_over_tls_only_when_its_connection_completed_a_handshake() {
    let (address, server, trusted) = serving(true).await;
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
    assert!(secure.ends_with("over-tls"), "{secure}");
    server.abort();

    let (address, server, _) = serving(false).await;
    let mut plain = tokio::net::TcpStream::connect(&address)
        .await
        .expect("connects");
    let clear = answer(&mut plain).await;
    assert!(clear.ends_with("in-clear"), "{clear}");
    server.abort();
}
