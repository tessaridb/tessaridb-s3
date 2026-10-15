//! Serving the internal surface: in the clear, or over mutual TLS where every connection must present a certificate
//! the cluster authority issued before a request is read.

use std::future::Future;
use std::sync::Arc;

use rustls::ServerConfig;
use tokio::net::TcpListener;

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
    crate::serve::serve(listener, internal_router(state), tls, shutdown, "internal").await
}
