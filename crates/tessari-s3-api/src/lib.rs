//! The S3 HTTP surface: request parsing, operation dispatch, authentication, and the error envelope.

mod audit;
mod console;
mod error;
mod idle_body;
mod internal;
mod metrics;
mod metrics_state;
mod pipeline;
mod principals;
pub mod routes;
mod serve;
mod state;
mod xml;

pub use audit::AuditWriter;
pub use console::{ConsoleState, console_router, serve_console};
pub use error::{Error, Result};
pub use internal::{Admit, InternalState, PeerCertificate, internal_router, serve_internal};
pub use metrics::{metrics_router, serve_metrics};
pub use serve::{OverTls, serve_s3};
pub use state::{ApiState, Clock};

/// The router: every request goes through the dispatch pipeline behind an in-flight limit that sheds load.
pub fn router(state: ApiState) -> axum::Router {
    let metrics = std::sync::Arc::clone(state.metrics());
    let limit = tower::ServiceBuilder::new()
        .layer(axum::error_handling::HandleErrorLayer::new(
            move |error: axum::BoxError| {
                let response = pipeline::refused(&metrics, &error);
                async move { response }
            },
        ))
        .load_shed()
        .concurrency_limit(state.max_inflight());
    axum::Router::new()
        .fallback(pipeline::handle)
        .layer(limit)
        .with_state(state)
}
