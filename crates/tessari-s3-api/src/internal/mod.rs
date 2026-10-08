//! The internal surface: the routes other nodes of the cluster call to store, read and remove this node's shards,
//! and to ask whether it is up. It
//! listens apart from the S3 surface, and every request — an unknown path included — must carry a signature made
//! with the cluster secret before anything is routed or touched.

mod auth;
mod params;
mod refusal;
mod serve;
mod shards;

pub use serve::serve_internal;

use std::sync::Arc;

use axum::routing::{get, put};
use tessari_s3_constants::{INTERNAL_DRIVE_PATH, INTERNAL_HEALTH_PATH, INTERNAL_SHARDS_PATH};
use tessari_s3_storage::Storage;
use tessari_s3_types::SecretKey;

use crate::Clock;

/// What every internal handler shares: the cluster secret, the clock and the storage services.
#[derive(Clone)]
pub struct InternalState {
    inner: Arc<Inner>,
}

struct Inner {
    secret: SecretKey,
    clock: Clock,
    storage: Storage,
    max_inflight: usize,
}

impl InternalState {
    /// State verifying requests with `secret` by `clock`, serving at most `max_inflight` at once.
    #[must_use]
    pub fn new(secret: SecretKey, clock: Clock, storage: Storage, max_inflight: usize) -> Self {
        Self {
            inner: Arc::new(Inner {
                secret,
                clock,
                storage,
                max_inflight,
            }),
        }
    }

    fn secret(&self) -> &[u8] {
        self.inner.secret.expose().as_bytes()
    }

    fn now(&self) -> i64 {
        (self.inner.clock)()
    }

    fn storage(&self) -> &Storage {
        &self.inner.storage
    }
}

/// The internal router: shard routes and the health route behind signature verification, behind an in-flight limit that sheds load.
pub fn internal_router(state: InternalState) -> axum::Router {
    let limit = tower::ServiceBuilder::new()
        .layer(axum::error_handling::HandleErrorLayer::new(refusal::shed))
        .load_shed()
        .concurrency_limit(state.inner.max_inflight);
    let route = format!("{INTERNAL_SHARDS_PATH}/{{id}}/{{index}}");
    axum::Router::new()
        .route(INTERNAL_HEALTH_PATH, get(answer))
        .route(INTERNAL_DRIVE_PATH, get(drive))
        .route(
            &route,
            put(shards::put_shard)
                .get(shards::get_shard)
                .delete(shards::delete_shard),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::authenticate,
        ))
        .layer(limit)
        .with_state(state)
}

/// `GET {INTERNAL_HEALTH_PATH}`: reached only through the signature check, so a 204 tells the calling node that this
/// one is up and shares its cluster secret, and tells anyone else nothing.
async fn answer() -> axum::http::StatusCode {
    axum::http::StatusCode::NO_CONTENT
}

/// `GET {INTERNAL_DRIVE_PATH}`: this node's data drive space in the drive form, for the console of whichever member an
/// operator signed in to; `404` on a node without a data directory.
async fn drive(
    axum::extract::State(state): axum::extract::State<InternalState>,
) -> Result<String, refusal::Refusal> {
    match state.storage().shards().drive().await? {
        Some(space) => Ok(space.to_wire()),
        None => Err(refusal::Refusal::NoDrive),
    }
}
