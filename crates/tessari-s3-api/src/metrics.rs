//! The metrics listener: `GET /metrics` answers the node's metrics as Prometheus text. It is a surface of its own,
//! never a route on the S3 listener, where `GET /metrics` is ListObjects of a bucket named `metrics`.

use std::future::Future;

use axum::Router;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;

use crate::ApiState;

/// The media type of the Prometheus text exposition format.
const TEXT_FORMAT: &str = "text/plain; version=0.0.4; charset=utf-8";

async fn scrape(State(state): State<ApiState>) -> Response {
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
