//! The internal shard surface a cluster member serves beside the S3 one.

use tessari_s3_api::{ApiState, InternalState, serve_internal};
use tessari_s3_infrastructure::S3Config;
use tessari_s3_infrastructure::tls::server_config;
use tessari_s3_storage::Storage;
use tokio::sync::watch;

/// On a node of a cluster, the internal shard surface: bound before anything is served, stopped with the S3
/// surface. `None` on a node of its own.
pub(crate) async fn surface(
    config: &S3Config,
    storage: &Storage,
    mut stop: watch::Receiver<bool>,
) -> anyhow::Result<Option<impl Future<Output = std::io::Result<()>> + use<>>> {
    let Some(cluster) = &config.cluster else {
        return Ok(None);
    };
    let state = InternalState::new(
        cluster.secret.clone(),
        ApiState::system_clock(),
        storage.clone(),
        config.max_inflight,
    );
    let tls = cluster.tls.as_ref().map(server_config).transpose()?;
    let listener = tokio::net::TcpListener::bind(cluster.internal_listen).await?;
    if tls.is_some() {
        tracing::info!(internal_listen = %cluster.internal_listen, "internal shard surface listening over mutual TLS");
    } else {
        tracing::warn!(
            internal_listen = %cluster.internal_listen,
            "internal shard surface listening in the clear: shard bytes cross the network unencrypted; set \
             TESSARIDB_S3_INTERNAL_TLS_CERT, _KEY and _CA"
        );
    }
    Ok(Some(serve_internal(listener, state, tls, async move {
        let _closed = stop.changed().await;
    })))
}
