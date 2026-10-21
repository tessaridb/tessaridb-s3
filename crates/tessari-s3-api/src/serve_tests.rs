use std::sync::Arc;

use axum::Extension;
use axum::routing::get;
use rustls::pki_types::ServerName;
use tessari_s3_infrastructure::tls::{ServerTls, listener_config};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use std::time::Duration;

use axum::body::Body;
use axum::routing::put;
use futures_util::StreamExt;

use super::{OverTls, Patience, serve, serve_with};

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

const QUICK: Patience = Patience {
    headers: Duration::from_millis(300),
    body_idle: Duration::from_millis(300),
    connections: 2,
};

/// A router whose `PUT /` answers how many body bytes arrived, or `stalled` when the body failed.
fn reader() -> axum::Router {
    axum::Router::new().route(
        "/",
        put(|body: Body| async move {
            let mut stream = body.into_data_stream();
            let mut bytes = 0_usize;
            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(chunk) => bytes = bytes.saturating_add(chunk.len()),
                    Err(_) => return "stalled".to_owned(),
                }
            }
            format!("read {bytes}")
        }),
    )
}

/// `router` served in the clear under [`QUICK`]: its address and the task serving it.
async fn quick(router: axum::Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a free port");
    let address = listener.local_addr().expect("bound").to_string();
    let server = tokio::spawn(async move {
        serve_with(
            listener,
            router,
            None,
            std::future::pending(),
            "test",
            &QUICK,
        )
        .await
        .expect("serves");
    });
    (address, server)
}

/// What the server sends before it closes, or `None` when it still holds the connection after `within`.
async fn closed_within(stream: &mut tokio::net::TcpStream, within: Duration) -> Option<String> {
    let mut bytes = Vec::new();
    match tokio::time::timeout(within, stream.read_to_end(&mut bytes)).await {
        Ok(_) => Some(String::from_utf8_lossy(&bytes).into_owned()),
        Err(_) => None,
    }
}

#[tokio::test]
async fn a_client_that_trickles_its_headers_is_closed() {
    let (address, server) = quick(echo()).await;
    let mut stream = tokio::net::TcpStream::connect(&address)
        .await
        .expect("connects");
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: local")
        .await
        .expect("half the headers");
    let seen = closed_within(&mut stream, Duration::from_secs(3)).await;
    assert!(seen.is_some(), "the connection is still held");
    assert!(
        !seen.unwrap_or_default().contains("in-clear"),
        "no request was served"
    );
    server.abort();
}

#[tokio::test]
async fn a_body_that_stops_arriving_fails_the_request() {
    let (address, server) = quick(reader()).await;
    let mut stream = tokio::net::TcpStream::connect(&address)
        .await
        .expect("connects");
    stream
        .write_all(
            b"PUT / HTTP/1.1\r\nHost: local\r\nContent-Length: 10\r\nConnection: close\r\n\r\nabc",
        )
        .await
        .expect("headers and a part of the body");
    let seen = closed_within(&mut stream, Duration::from_secs(3)).await;
    assert!(
        seen.as_deref()
            .is_some_and(|answer| answer.ends_with("stalled")),
        "{seen:?}"
    );
    server.abort();
}

#[tokio::test]
async fn a_slow_but_steady_body_is_read_whole() {
    let (address, server) = quick(reader()).await;
    let mut stream = tokio::net::TcpStream::connect(&address)
        .await
        .expect("connects");
    stream
        .write_all(
            b"PUT / HTTP/1.1\r\nHost: local\r\nContent-Length: 4\r\nConnection: close\r\n\r\n",
        )
        .await
        .expect("headers");
    for piece in [b"a", b"b", b"c", b"d"] {
        tokio::time::sleep(Duration::from_millis(150)).await;
        stream.write_all(piece).await.expect("a piece");
    }
    let seen = closed_within(&mut stream, Duration::from_secs(3)).await;
    assert!(
        seen.as_deref()
            .is_some_and(|answer| answer.ends_with("read 4")),
        "{seen:?}"
    );
    server.abort();
}

#[tokio::test]
async fn a_listener_holds_at_most_its_connections_and_takes_the_next_when_one_closes() {
    let (address, server) = quick(echo()).await;
    let mut held = Vec::new();
    for _ in 0..2 {
        held.push(
            tokio::net::TcpStream::connect(&address)
                .await
                .expect("connects"),
        );
    }
    // Both slots taken by connections that send nothing: a third is not served until one goes.
    let mut third = tokio::net::TcpStream::connect(&address)
        .await
        .expect("the kernel accepts it");
    third.write_all(REQUEST).await.expect("request sent");
    let mut first = [0_u8; 1];
    let early = tokio::time::timeout(Duration::from_millis(150), third.read(&mut first)).await;
    assert!(early.is_err(), "served beyond the connection limit");
    drop(held);
    let seen = closed_within(&mut third, Duration::from_secs(3)).await;
    assert!(
        seen.as_deref()
            .is_some_and(|answer| answer.ends_with("in-clear")),
        "{seen:?}"
    );
    server.abort();
}
