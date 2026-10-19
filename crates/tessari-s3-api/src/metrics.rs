//! The metrics listener: `GET /metrics` answers the node's metrics as Prometheus text, the heal and replication
//! backlogs and the cluster's members — probed — read at that moment, each under its own deadline. It is a surface of its own, never a route on the S3
//! listener, where `GET /metrics` is ListObjects of a bucket named `metrics`.

use std::future::Future;
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;

use futures_util::{StreamExt, stream};
use tessari_s3_constants::observability::{METRICS_BACKLOG_COUNT_MAX, METRICS_COLLECT_SECS};
use tessari_s3_constants::{CONSOLE_MEMBER_PROBE_SECS, ERASURE_MAX_WIDTH};
use tokio::time::timeout;

use crate::ApiState;

/// The media type of the Prometheus text exposition format.
const TEXT_FORMAT: &str = "text/plain; version=0.0.4; charset=utf-8";

/// Reads the backlogs into the gauges: the heal backlog only on a cluster, where data is healed at all.
async fn collect(state: &ApiState) {
    let metrics = state.metrics();
    match timeout(Duration::from_secs(METRICS_COLLECT_SECS), quorum(state)).await {
        Ok(Ok(found)) => {
            if let Some((members, answering)) = found {
                metrics.cluster(members, answering);
            }
            metrics.collected("cluster", true);
        }
        Ok(Err(error)) => {
            tracing::warn!(error = %error, "the cluster's members could not be read");
            metrics.collected("cluster", false);
        }
        Err(_) => {
            tracing::warn!("the cluster's members were not probed in time");
            metrics.collected("cluster", false);
        }
    }
    let deadline = Duration::from_secs(METRICS_COLLECT_SECS);
    let objects = state.storage().objects();
    metrics.verification_failures(objects.verification_failures());
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

/// The cluster's members and how many answer a probe; `None` off a cluster, which has none.
async fn quorum(state: &ApiState) -> tessari_s3_storage::Result<Option<(u64, u64)>> {
    let cluster = state.storage().cluster();
    let members = cluster.members().await?;
    if members.is_empty() {
        return Ok(None);
    }
    let budget = Duration::from_secs(CONSOLE_MEMBER_PROBE_SECS);
    let count = members.len();
    let answering = stream::iter(members)
        .map(|member| {
            let cluster = cluster.clone();
            async move { cluster.answers(&member, budget).await }
        })
        .buffer_unordered(usize::from(ERASURE_MAX_WIDTH))
        .filter(|answered| std::future::ready(*answered))
        .count()
        .await;
    Ok(Some((
        u64::try_from(count).unwrap_or(u64::MAX),
        u64::try_from(answering).unwrap_or(u64::MAX),
    )))
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
