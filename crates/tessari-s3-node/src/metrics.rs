//! The metrics a node serves as Prometheus text when `TESSARIDB_S3_METRICS_LISTEN` is set.

use tessari_s3_api::{ApiState, serve_metrics};
use tessari_s3_infrastructure::S3Config;
use tokio::sync::watch;

/// The metrics listener, bound before anything is served and stopped with the other surfaces; `None` when not
/// configured.
pub(crate) async fn surface(
    config: &S3Config,
    state: &ApiState,
    mut stop: watch::Receiver<bool>,
) -> anyhow::Result<Option<impl Future<Output = std::io::Result<()>> + use<>>> {
    let Some(listen) = config.metrics_listen else {
        return Ok(None);
    };
    let listener = tokio::net::TcpListener::bind(listen).await?;
    tracing::info!(metrics_listen = %listen, "metrics listening");
    Ok(Some(serve_metrics(listener, state.clone(), async move {
        let _closed = stop.changed().await;
    })))
}
