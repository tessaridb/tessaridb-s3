//! The operator console's surface (ADR-0006): its own listener, a JSON API under `/api/v1` where every route but
//! signing in requires a session, and the page itself, which carries no data and is served without one. Writes go through the same storage
//! services the S3 handlers call.

mod actions;
mod buckets;
mod content;
mod error;
mod guard;
mod input;
mod limits;
mod objects;
mod page;
mod session;
mod status;
mod usage;

use std::sync::Arc;

use axum::middleware::from_fn_with_state;
use axum::routing::{delete, get, post};
use tessari_s3_infrastructure::S3Config;
use tessari_s3_storage::Storage;
use tessari_s3_types::SecretKey;

use crate::Clock;
use error::ConsoleError;
use limits::Window;

/// What every console handler shares.
#[derive(Clone)]
pub struct ConsoleState {
    inner: Arc<Inner>,
}

struct Inner {
    key_id: String,
    secret: SecretKey,
    clock: Clock,
    storage: Storage,
    secure_cookie: bool,
    sign_ins: Window,
    requests: Window,
    node: Option<String>,
    region: String,
    erasure: Option<String>,
}

impl ConsoleState {
    /// State for the console `config` describes, timed by `clock`; `None` when no console is configured.
    #[must_use]
    pub fn new(config: &S3Config, clock: Clock, storage: Storage) -> Option<Self> {
        let console = config.console.as_ref()?;
        Some(Self {
            inner: Arc::new(Inner {
                key_id: config.root_access_key.clone(),
                secret: config.root_secret_key.clone(),
                clock,
                storage,
                secure_cookie: console.tls.is_some(),
                sign_ins: Window::new(console.sign_ins_per_minute),
                requests: Window::new(console.requests_per_minute),
                node: config
                    .cluster
                    .as_ref()
                    .map(|cluster| cluster.node.as_str().to_owned()),
                region: config.region.clone(),
                erasure: config
                    .cluster
                    .as_ref()
                    .map(|cluster| format!("{}+{}", cluster.code.data(), cluster.code.parity())),
            }),
        })
    }

    fn key_id(&self) -> &str {
        &self.inner.key_id
    }

    fn secret(&self) -> &[u8] {
        self.inner.secret.expose().as_bytes()
    }

    fn now(&self) -> i64 {
        (self.inner.clock)()
    }

    fn secure_cookie(&self) -> bool {
        self.inner.secure_cookie
    }

    fn sign_ins(&self) -> &Window {
        &self.inner.sign_ins
    }

    fn requests(&self) -> &Window {
        &self.inner.requests
    }

    fn node(&self) -> Option<&str> {
        self.inner.node.as_deref()
    }

    fn region(&self) -> &str {
        &self.inner.region
    }

    fn erasure(&self) -> Option<&str> {
        self.inner.erasure.as_deref()
    }

    fn storage(&self) -> &Storage {
        &self.inner.storage
    }
}

/// Serves the console on `listener` until `shutdown` completes, over TLS when `tls` is given.
///
/// # Errors
/// The listener's failure.
pub async fn serve_console(
    listener: tokio::net::TcpListener,
    state: ConsoleState,
    tls: Option<Arc<rustls::ServerConfig>>,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    crate::serve::serve(listener, console_router(state), tls, shutdown, "console").await
}

async fn not_found() -> ConsoleError {
    ConsoleError::not_found()
}

/// The console router: signing in and the page's assets are the routes without a session; every other route, an
/// unknown path included, is behind the session guard.
pub fn console_router(state: ConsoleState) -> axum::Router {
    let protected = axum::Router::new()
        .route("/api/v1/status", get(status::status))
        .route("/api/v1/session", delete(session::sign_out))
        .route("/api/v1/buckets", get(buckets::list).post(buckets::create))
        .route("/api/v1/buckets/{bucket}", delete(buckets::delete))
        .route("/api/v1/buckets/{bucket}/objects", get(objects::list))
        .route(
            "/api/v1/buckets/{bucket}/object",
            get(objects::detail).delete(objects::delete),
        )
        .route(
            "/api/v1/buckets/{bucket}/object/content",
            get(content::download),
        )
        .route("/api/v1/actions", get(actions::list))
        .route("/api/v1/usage", get(usage::usage))
        .fallback(not_found)
        .layer(from_fn_with_state(state.clone(), guard::require_session));
    axum::Router::new()
        .route("/api/v1/session", post(session::sign_in))
        .merge(page::routes())
        .merge(protected)
        .with_state(state)
}
