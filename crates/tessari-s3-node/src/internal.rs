//! The internal shard surface a cluster member serves beside the S3 one.

use tessari_s3_api::{ApiState, InternalState, internal_router};
use tessari_s3_infrastructure::S3Config;
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
    let listener = tokio::net::TcpListener::bind(cluster.internal_listen).await?;
    tracing::info!(internal_listen = %cluster.internal_listen, "internal shard surface listening");
    let serving =
        axum::serve(listener, internal_router(state)).with_graceful_shutdown(async move {
            let _closed = stop.changed().await;
        });
    Ok(Some(serving.into_future()))
}
