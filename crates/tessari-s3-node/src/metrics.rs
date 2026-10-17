//! The metrics a node serves as Prometheus text when `TESSARIDB_S3_METRICS_LISTEN` is set, and the access log it
//! keeps unless `TESSARIDB_S3_AUDIT` is `off`.

use tessari_s3_api::{ApiState, serve_metrics};
use tessari_s3_constants::observability::AUDIT_QUEUE_MAX;
use tessari_s3_daemons::Runs;
use tessari_s3_infrastructure::S3Config;
use tokio::sync::watch;
use tracing::Instrument;

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

/// The access-log writer, stopped with the daemons once it has landed its queue; `None` when switched off.
pub(crate) fn access_log(
    config: &S3Config,
    state: &ApiState,
    stop: watch::Receiver<bool>,
) -> Option<impl Future<Output = Runs> + use<>> {
    if !config.audit {
        tracing::info!("access log off");
        return None;
    }
    let writer = state.audit_writer(AUDIT_QUEUE_MAX)?;
    let writer = writer
        .run(stop)
        .instrument(tracing::info_span!("access_log"));
    // Not an interval daemon: it runs no passes to report.
    Some(async move {
        writer.await;
        Runs::default()
    })
}
