//! `GET /api/v1/status`: what this node is — its name in the cluster, the build, the region it signs for.

use axum::Json;
use axum::extract::State;
use serde::Serialize;

use super::ConsoleState;

#[derive(Serialize)]
pub(super) struct Status {
    version: &'static str,
    node: Option<String>,
    region: String,
    erasure: Option<String>,
}

pub(super) async fn status(State(state): State<ConsoleState>) -> Json<Status> {
    Json(Status {
        version: env!("CARGO_PKG_VERSION"),
        node: state.node().map(str::to_owned),
        region: state.region().to_owned(),
        erasure: state.erasure().map(str::to_owned),
    })
}
