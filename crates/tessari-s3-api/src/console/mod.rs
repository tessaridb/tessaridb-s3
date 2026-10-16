//! The operator console's surface (ADR-0006): its own listener, a JSON API under `/api/v1` where every route but
//! signing in requires a session, and the page itself, which carries no data and is served without one. Writes go through the same storage
//! services the S3 handlers call. Any live access key signs in; what it may see and do is decided per request by the
//! same evaluator the S3 path uses.

mod access;
mod acknowledgement;
mod actions;
mod break_glass;
mod bucket_encryption;
mod buckets;
mod cluster;
mod content;
mod encryption;
mod encryption_rotation;
mod error;
mod grants;
mod guard;
mod identity;
mod input;
mod keys;
mod layout;
mod limits;
mod logout;
mod media;
mod media_failed;
mod members;
mod objects;
mod oidc;
mod page;
mod quotas;
mod regions;
mod session;
mod spaces;
mod status;
mod upload_keys;
mod usage;
mod users;

use std::sync::Arc;

use axum::middleware::from_fn_with_state;
use axum::routing::{delete, get, post, put};
use tessari_s3_constants::CONSOLE_OIDC_REFRESH_MIN_SECS;
use tessari_s3_core::authz::Principal;
use tessari_s3_infrastructure::oidc::OidcClient;
use tessari_s3_infrastructure::{S3Config, WebhookClient};
use tessari_s3_storage::Storage;
use tessari_s3_types::{Code, SecretKey};

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
    code: Option<Code>,
    meta_addresses: Vec<String>,
    principals: Principals,
    usage_interval_secs: u64,
    oidc: Option<OidcClient>,
    /// Where a root sign-in is announced, when an identity provider and a webhook are configured.
    break_glass_alert: Option<(WebhookClient, String)>,
}

impl ConsoleState {
    /// State for the console `config` describes, timed by `clock`; `None` when no console is configured.
    ///
    /// # Errors
    /// [`tessari_s3_infrastructure::Error::InvalidConfig`] when the identity provider's client cannot be built.
    pub fn new(
        config: &S3Config,
        clock: Clock,
        storage: Storage,
    ) -> Result<Option<Self>, tessari_s3_infrastructure::Error> {
        let Some(console) = config.console.as_ref() else {
            return Ok(None);
        };
        let oidc = console
            .oidc
            .as_ref()
            .map(|settings| OidcClient::new(settings, CONSOLE_OIDC_REFRESH_MIN_SECS))
            .transpose()?;
        let break_glass_alert = break_glass::alert(console.oidc.as_ref())?;
        Ok(Some(Self {
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
                code: config.cluster.as_ref().map(|cluster| cluster.code),
                meta_addresses: config.meta.addresses.clone(),
                principals: Principals::new(),
                usage_interval_secs: config.usage_interval_secs,
                oidc,
                break_glass_alert,
            }),
        }))
    }

    /// The identity provider's client, when one is configured.
    fn oidc(&self) -> Option<&OidcClient> {
        self.inner.oidc.as_ref()
    }

    /// Where a root sign-in is announced, if anywhere.
    fn break_glass_alert(&self) -> Option<&(WebhookClient, String)> {
        self.inner.break_glass_alert.as_ref()
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
        // A one-key upload credential signs nothing in here: it is for uploading its key over S3 and nothing else.
        Ok(live
            .filter(|live| live.scope.is_none())
            .map(|live| (Principal::User(live.principal), live.secret)))
    }

    /// The principal of the user `name` as the store holds it now, never cached: `None` for a name that is not a
    /// user's, or a disabled user.
    async fn user_principal(&self, name: &str) -> Result<Option<Principal>, ConsoleError> {
        let Some(name) = tessari_s3_core::authz::UserName::new(name) else {
            return Ok(None);
        };
        Ok(self
            .inner
            .storage
            .users()
            .principal(&name)
            .await?
            .map(Principal::User))
    }

    fn secret(&self) -> &[u8] {
        self.inner.secret.expose().as_bytes()
    }

    fn now(&self) -> i64 {
        (self.inner.clock)()
    }

    fn usage_interval_secs(&self) -> u64 {
        self.inner.usage_interval_secs
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

    fn code(&self) -> Option<Code> {
        self.inner.code
    }

    fn meta_addresses(&self) -> &[String] {
        &self.inner.meta_addresses
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

/// The console router: signing in (with a key, or through the identity provider) and the page's assets are the routes
/// without a session; every other route, an
/// unknown path included, is behind the session guard.
pub fn console_router(state: ConsoleState) -> axum::Router {
    let protected = axum::Router::new()
        .route("/api/v1/status", get(status::status))
        .route(
            "/api/v1/session",
            get(session::capabilities).delete(session::sign_out),
        )
        .route("/api/v1/cluster", get(cluster::cluster))
        .route(
            "/api/v1/cluster/layout/staged",
            get(layout::staged)
                .put(layout::stage)
                .delete(layout::discard),
        )
        .route("/api/v1/cluster/layout/apply", post(layout::apply))
        .route("/api/v1/cluster/members", get(members::list))
        .route("/api/v1/cluster/members/approve", post(members::approve))
        .route("/api/v1/cluster/members/token", post(members::token))
        .route("/api/v1/cluster/members/remove", post(members::remove))
        .route("/api/v1/encryption", get(encryption::state))
        .route("/api/v1/encryption/unseal", post(encryption::unseal))
        .route("/api/v1/encryption/seal", post(encryption::seal))
        .route("/api/v1/encryption/keys", post(encryption::create))
        .route(
            "/api/v1/encryption/rotate",
            post(encryption_rotation::rotate),
        )
        .route(
            "/api/v1/encryption/destroy",
            post(encryption_rotation::destroy),
        )
        .route("/api/v1/regions", get(regions::list))
        .route("/api/v1/regions/failover", post(regions::fail_over))
        .route("/api/v1/regions/restore", post(regions::restore))
        .route("/api/v1/buckets", get(buckets::list).post(buckets::create))
        .route("/api/v1/buckets/{bucket}", delete(buckets::delete))
        .route("/api/v1/buckets/{bucket}/quota", put(quotas::set))
        .route(
            "/api/v1/buckets/{bucket}/acknowledgement",
            put(acknowledgement::set),
        )
        .route(
            "/api/v1/buckets/{bucket}/encryption",
            put(bucket_encryption::set),
        )
        .route(
            "/api/v1/buckets/{bucket}/media",
            get(media::get).put(media::set),
        )
        .route(
            "/api/v1/buckets/{bucket}/upload-keys",
            post(upload_keys::issue),
        )
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
        .route("/api/v1/media/failed", get(media_failed::list))
        .route("/api/v1/spaces", get(spaces::list).post(spaces::create))
        .route("/api/v1/users", get(users::list).post(users::create))
        .route("/api/v1/users/{user}/disabled", put(users::set_disabled))
        .route(
            "/api/v1/users/{user}/identity",
            put(identity::link).delete(identity::unlink),
        )
        .route(
            "/api/v1/users/{user}/keys",
            get(keys::list).post(keys::issue),
        )
        .route("/api/v1/users/{user}/keys/{key}", delete(keys::revoke))
        .route(
            "/api/v1/users/{user}/grants/{bucket}",
            put(grants::set).delete(grants::remove),
        )
        .fallback(not_found)
        .layer(from_fn_with_state(state.clone(), guard::require_session));
    axum::Router::new()
        .route("/api/v1/session", post(session::sign_in))
        .merge(oidc::routes())
        .merge(page::routes())
        .merge(protected)
        .with_state(state)
}
