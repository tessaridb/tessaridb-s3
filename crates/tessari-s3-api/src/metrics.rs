//! The metrics listener: `GET /metrics` answers the node's metrics as Prometheus text, the heal and replication
//! backlogs read at that moment, each under its own deadline. It is a surface of its own, never a route on the S3
//! listener, where `GET /metrics` is ListObjects of a bucket named `metrics`.

use std::future::Future;
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;

use tessari_s3_constants::observability::{METRICS_BACKLOG_COUNT_MAX, METRICS_COLLECT_SECS};
use tokio::time::timeout;

use crate::ApiState;

/// The media type of the Prometheus text exposition format.
const TEXT_FORMAT: &str = "text/plain; version=0.0.4; charset=utf-8";

/// Reads the backlogs into the gauges: the heal backlog only on a cluster, where data is healed at all.
async fn collect(state: &ApiState) {
    let metrics = state.metrics();
    let deadline = Duration::from_secs(METRICS_COLLECT_SECS);
    let objects = state.storage().objects();
    match timeout(deadline, objects.heal_backlog(METRICS_BACKLOG_COUNT_MAX)).await {
        Ok(Ok(None)) => {}
        Ok(Ok(Some(backlog))) => {
            metrics.heal(
                u64::try_from(backlog.listed).unwrap_or(u64::MAX),
                u64::try_from(backlog.at_risk).unwrap_or(u64::MAX),
            );
            metrics.collected("heal", true);
        }
        Ok(Err(error)) => {
            tracing::warn!(error = %error, "the heal backlog could not be read");
            metrics.collected("heal", false);
        }
        Err(_) => {
            tracing::warn!("the heal backlog was not read in time");
            metrics.collected("heal", false);
        }
    }
    match timeout(deadline, state.storage().replication().progress()).await {
        Ok(Ok(progress)) => {
            metrics.replication(progress.backlog, progress.in_flight, progress.failed);
            metrics.collected("replication", true);
        }
        Ok(Err(error)) => {
            tracing::warn!(error = %error, "the replication backlog could not be read");
            metrics.collected("replication", false);
        }
        Err(_) => {
            tracing::warn!("the replication backlog was not read in time");
            metrics.collected("replication", false);
        }
    }
}

async fn scrape(State(state): State<ApiState>) -> Response {
    collect(&state).await;
    match state.metrics().encode() {
        Ok(text) => ([(header::CONTENT_TYPE, TEXT_FORMAT)], text).into_response(),
        Err(error) => {
            tracing::error!(error = %error, "the metrics could not be encoded");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// The metrics router: `GET /metrics`, and nothing else.
pub fn metrics_router(state: ApiState) -> Router {
    Router::new()
        .route("/metrics", get(scrape))
        .with_state(state)
}

/// Serves the metrics on `listener` until `shutdown` completes.
///
/// # Errors
/// The listener's failure.
pub async fn serve_metrics(
    listener: tokio::net::TcpListener,
    state: ApiState,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    crate::serve::serve(listener, metrics_router(state), None, shutdown, "metrics").await
}
