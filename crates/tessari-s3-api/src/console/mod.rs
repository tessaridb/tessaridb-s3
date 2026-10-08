//! The operator console's surface (ADR-0006): its own listener, a JSON API under `/api/v1` where every route but
//! signing in requires a session, and the page itself, which carries no data and is served without one. Writes go through the same storage
//! services the S3 handlers call. Any live access key signs in; what it may see and do is decided per request by the
//! same evaluator the S3 path uses.

mod access;
mod actions;
mod buckets;
mod content;
mod error;
mod grants;
mod guard;
mod input;
mod keys;
mod limits;
mod objects;
mod page;
mod session;
mod spaces;
mod status;
mod usage;
mod users;

use std::sync::Arc;

use axum::middleware::from_fn_with_state;
use axum::routing::{delete, get, post, put};
use tessari_s3_core::authz::Principal;
use tessari_s3_infrastructure::S3Config;
use tessari_s3_storage::Storage;
use tessari_s3_types::SecretKey;

use crate::Clock;
use crate::principals::{Principals, resolve_user_key};
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
    principals: Principals,
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
                principals: Principals::new(),
            }),
        })
    }

    fn key_id(&self) -> &str {
        &self.inner.key_id
    }

    /// Who signs in with `key_id`, and that key's secret: the root credential from configuration, a user's key
    /// through the principal cache (5 s); `None` for a key that does not resolve.
    async fn principal_for(
        &self,
        key_id: &str,
    ) -> Result<Option<(Principal, SecretKey)>, ConsoleError> {
        if key_id == self.key_id() {
            return Ok(Some((Principal::Root, self.inner.secret.clone())));
        }
        let live = resolve_user_key(
            &self.inner.principals,
            self.inner.storage.users(),
            key_id,
            self.now(),
        )
        .await?;
        Ok(live.map(|live| (Principal::User(live.principal), live.secret)))
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
        .route("/api/v1/spaces", get(spaces::list).post(spaces::create))
        .route("/api/v1/users", get(users::list).post(users::create))
        .route("/api/v1/users/{user}/disabled", put(users::set_disabled))
        .route("/api/v1/users/{user}/keys", post(keys::issue))
        .route(
            "/api/v1/users/{user}/grants/{bucket}",
            put(grants::set).delete(grants::remove),
        )
        .fallback(not_found)
        .layer(from_fn_with_state(state.clone(), guard::require_session));
    axum::Router::new()
        .route("/api/v1/session", post(session::sign_in))
        .merge(page::routes())
        .merge(protected)
        .with_state(state)
}
