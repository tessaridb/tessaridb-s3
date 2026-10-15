//! The internal shard surface a cluster member serves beside the S3 one.

use std::sync::Arc;

use tessari_s3_api::{ApiState, InternalState, serve_internal};
use tessari_s3_infrastructure::S3Config;
use tessari_s3_infrastructure::tls::server_config;
use tessari_s3_storage::Storage;
use tokio::sync::watch;

/// On a node of a cluster, the internal shard surface over mutual TLS, admitting the cluster's members: bound before
/// anything is served, stopped with the S3 surface. `None` on a node of its own.
///
/// # Errors
/// Its TLS material does not load, or the address cannot be bound.
pub(crate) async fn surface(
    config: &S3Config,
    storage: &Storage,
    mut stop: watch::Receiver<bool>,
) -> anyhow::Result<Option<impl Future<Output = std::io::Result<()>> + use<>>> {
    let Some(cluster) = &config.cluster else {
        return Ok(None);
    };
    let state = InternalState::new(
        ApiState::system_clock(),
        storage.clone(),
        Arc::new(storage.members().clone()),
        config.max_inflight,
    );
    let tls = server_config(&cluster.tls)?;
    let listener = tokio::net::TcpListener::bind(cluster.internal_listen).await?;
    tracing::info!(internal_listen = %cluster.internal_listen, "internal shard surface listening over mutual TLS");
    Ok(Some(serve_internal(
        listener,
        state,
        Some(tls),
        async move {
            let _closed = stop.changed().await;
        },
    )))
}
