//! Serving a surface on a listener: in the clear, or over TLS where the handshake must complete before a request is
//! read — each connection's handshake in its own tracked task under a deadline, so a slow client holds only itself.
//! Handlers see the client's address as `ConnectInfo` either way. On shutdown the listener stops accepting and the
//! connections in flight finish.

use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ConnectInfo;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder;
use hyper_util::server::graceful::GracefulShutdown;
use hyper_util::service::TowerToHyperService;
use rustls::ServerConfig;
use tessari_s3_constants::INTERNAL_TLS_HANDSHAKE_SECS;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio_rustls::TlsAcceptor;

/// Serves `router` on `listener` until `shutdown` completes, over TLS when `tls` is given; `surface` names it in the
/// log.
///
/// # Errors
/// The listener's failure.
pub(crate) async fn serve(
    listener: TcpListener,
    router: axum::Router,
    tls: Option<Arc<ServerConfig>>,
    shutdown: impl Future<Output = ()> + Send + 'static,
    surface: &'static str,
) -> std::io::Result<()> {
    let Some(tls) = tls else {
        return axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(shutdown)
        .await;
    };
    let acceptor = TlsAcceptor::from(tls);
    let builder = Builder::new(TokioExecutor::new());
    let graceful = GracefulShutdown::new();
    let mut connections = JoinSet::new();
    tokio::pin!(shutdown);
    loop {
        // Finished connections are reaped as the loop turns, so the set holds only live ones.
        while connections.try_join_next().is_some() {}
        let (stream, peer) = tokio::select! {
            biased;
            () = &mut shutdown => break,
            accepted = listener.accept() => match accepted {
                Ok(accepted) => accepted,
                Err(error) => {
                    tracing::warn!(surface, error = %error, "a connection could not be accepted");
                    continue;
                }
            },
        };
        let (acceptor, builder, watcher) = (acceptor.clone(), builder.clone(), graceful.watcher());
        let router = router.clone().layer(axum::Extension(ConnectInfo(peer)));
        // The handshake runs in the connection's own task: a client slow to finish it holds only itself.
        connections.spawn(async move {
            let handshake = tokio::time::timeout(
                Duration::from_secs(INTERNAL_TLS_HANDSHAKE_SECS),
                acceptor.accept(stream),
            )
            .await;
            let Ok(Ok(stream)) = handshake else {
                tracing::warn!(surface, %peer, "a connection did not complete its TLS handshake");
                return;
            };
            let connection = builder
                .serve_connection_with_upgrades(
                    TokioIo::new(stream),
                    TowerToHyperService::new(router),
                )
                .into_owned();
            if let Err(error) = watcher.watch(connection).await {
                tracing::debug!(surface, %peer, error = %error, "a connection ended with an error");
            }
        });
    }
    graceful.shutdown().await;
    while connections.join_next().await.is_some() {}
    Ok(())
}
