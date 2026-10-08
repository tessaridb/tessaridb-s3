//! The internal surface: the routes other nodes of the cluster call to store, read and remove this node's shards. It
//! listens apart from the S3 surface, and every request — an unknown path included — must carry a signature made
//! with the cluster secret before anything is routed or touched.

mod auth;
mod params;
mod refusal;
mod serve;
mod shards;

pub use serve::serve_internal;

use std::sync::Arc;

use axum::routing::put;
use tessari_s3_constants::INTERNAL_SHARDS_PATH;
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

/// The internal router: shard routes behind signature verification, behind an in-flight limit that sheds load.
pub fn internal_router(state: InternalState) -> axum::Router {
    let limit = tower::ServiceBuilder::new()
        .layer(axum::error_handling::HandleErrorLayer::new(refusal::shed))
        .load_shed()
        .concurrency_limit(state.inner.max_inflight);
    let route = format!("{INTERNAL_SHARDS_PATH}/{{id}}/{{index}}");
    axum::Router::new()
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
