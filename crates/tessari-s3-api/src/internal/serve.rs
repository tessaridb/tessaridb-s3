//! Serving the internal surface: in the clear, or over mutual TLS where every connection must present a certificate
//! the cluster authority issued before a request is read.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder;
use hyper_util::server::graceful::GracefulShutdown;
use hyper_util::service::TowerToHyperService;
use rustls::ServerConfig;
use tessari_s3_constants::INTERNAL_TLS_HANDSHAKE_SECS;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio_rustls::TlsAcceptor;

use super::{InternalState, internal_router};

/// Serves the internal routes on `listener` until `shutdown` completes, over TLS when `tls` is given; then stops
/// accepting and lets the connections in flight finish.
///
/// # Errors
/// The listener's failure.
pub async fn serve_internal(
    listener: TcpListener,
    state: InternalState,
    tls: Option<Arc<ServerConfig>>,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    let router = internal_router(state);
    let Some(tls) = tls else {
        return axum::serve(listener, router)
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
                    tracing::warn!(error = %error, "the internal surface could not accept a connection");
                    continue;
                }
            },
        };
        let (acceptor, builder, router, watcher) = (
            acceptor.clone(),
            builder.clone(),
            router.clone(),
            graceful.watcher(),
        );
        // The handshake runs in the connection's own task: a peer slow to finish it holds only itself.
        connections.spawn(async move {
            let handshake = tokio::time::timeout(
                Duration::from_secs(INTERNAL_TLS_HANDSHAKE_SECS),
                acceptor.accept(stream),
            )
            .await;
            let Ok(Ok(stream)) = handshake else {
                tracing::warn!(%peer, "an internal connection did not complete a mutual TLS handshake");
                return;
            };
            let connection = builder
                .serve_connection_with_upgrades(TokioIo::new(stream), TowerToHyperService::new(router))
                .into_owned();
            if let Err(error) = watcher.watch(connection).await {
                tracing::debug!(%peer, error = %error, "an internal connection ended with an error");
            }
        });
    }
    graceful.shutdown().await;
    while connections.join_next().await.is_some() {}
    Ok(())
}
