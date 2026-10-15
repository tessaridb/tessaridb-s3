//! The internal surface: the routes other nodes of the cluster call to store, read and remove this node's shards,
//! and to ask whether it is up. It listens apart from the S3 surface, over mutual TLS, and every request — an unknown
//! path included — must come from an admitted member presenting its own certificate before anything is routed or
//! touched.

mod admit;
mod params;
mod refusal;
mod serve;
mod shards;

pub use admit::{Admit, PeerCertificate};
pub use serve::serve_internal;

use std::sync::Arc;

use axum::routing::{get, put};
use tessari_s3_constants::{INTERNAL_DRIVE_PATH, INTERNAL_HEALTH_PATH, INTERNAL_SHARDS_PATH};
use tessari_s3_storage::Storage;
use tessari_s3_storage::members::Admission;

use crate::Clock;

/// What every internal handler shares: the membership it admits by, the answers it keeps, the clock and the storage
/// services.
#[derive(Clone)]
pub struct InternalState {
    inner: Arc<Inner>,
}

struct Inner {
    admit: Arc<dyn Admit>,
    /// Membership answers by (node, certificate fingerprint), with the second they were read.
    admissions: scc::HashMap<(String, String), (Admission, i64)>,
    clock: Clock,
    storage: Storage,
    max_inflight: usize,
}

impl InternalState {
    /// State admitting by `admit` at the time `clock` gives, serving at most `max_inflight` requests at once.
    #[must_use]
    pub fn new(clock: Clock, storage: Storage, admit: Arc<dyn Admit>, max_inflight: usize) -> Self {
        Self {
            inner: Arc::new(Inner {
                admit,
                admissions: scc::HashMap::new(),
                clock,
                storage,
                max_inflight,
            }),
        }
    }

    fn admit(&self) -> &dyn Admit {
        self.inner.admit.as_ref()
    }

    fn admissions(&self) -> &scc::HashMap<(String, String), (Admission, i64)> {
        &self.inner.admissions
    }

    fn now(&self) -> i64 {
        (self.inner.clock)()
    }

    fn storage(&self) -> &Storage {
        &self.inner.storage
    }
}

/// The internal router: shard routes and the health route behind admission, behind an in-flight limit that sheds load.
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
            admit::admit,
        ))
        .layer(limit)
        .with_state(state)
}

/// `GET {INTERNAL_HEALTH_PATH}`: reached only through admission, so a 204 tells an admitted member that this node is
/// up, and tells anyone else nothing.
async fn answer() -> axum::http::StatusCode {
    axum::http::StatusCode::NO_CONTENT
}

/// `GET {INTERNAL_DRIVE_PATH}`: this node's data drive space in the drive form, for the console of whichever member an
/// operator signed in to.
async fn drive(
    axum::extract::State(state): axum::extract::State<InternalState>,
) -> Result<String, refusal::Refusal> {
    Ok(state.storage().shards().drive().await?.to_wire())
}
