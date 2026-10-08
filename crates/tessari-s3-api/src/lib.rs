//! The S3 HTTP surface: request parsing, operation dispatch, authentication, and the error envelope.

mod console;
mod error;
mod internal;
mod pipeline;
mod principals;
pub mod routes;
mod serve;
mod state;
mod xml;

pub use console::{ConsoleState, console_router, serve_console};
pub use error::{Error, Result};
pub use internal::{InternalState, internal_router, serve_internal};
pub use state::{ApiState, Clock};

/// The router: every request goes through the dispatch pipeline behind an in-flight limit that sheds load.
pub fn router(state: ApiState) -> axum::Router {
    let limit = tower::ServiceBuilder::new()
        .layer(axum::error_handling::HandleErrorLayer::new(
            pipeline::refused,
        ))
        .load_shed()
        .concurrency_limit(state.max_inflight());
    axum::Router::new()
        .fallback(pipeline::handle)
        .layer(limit)
        .with_state(state)
}
