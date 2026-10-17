//! What every request handler shares: the signing region, the endpoint domains, the root credential, the cache of
//! resolved user keys, the clock, the node's metrics and where requests queue their access-log records.

use std::sync::{Arc, OnceLock};

use tessari_s3_core::auth::{session_token_expiry, session_token_matches};
use tessari_s3_core::authz::Principal;
use tessari_s3_core::objects::token::TokenKey;
use tessari_s3_infrastructure::S3Config;
use tessari_s3_observability::Metrics;
use tessari_s3_storage::Storage;
use tessari_s3_storage::audit::AccessRecord;
use tessari_s3_types::{ErrorCode, SecretKey};

use crate::principals::{Principals, principal_of, resolve_user_key};
use crate::{Error, Result};

/// Who signs with an access key, and the secret the signature is verified with.
pub(crate) struct Credential {
    /// The root credential or a user.
    pub(crate) principal: Principal,
    /// The key's secret.
    pub(crate) secret: SecretKey,
}

/// The clock requests are judged by, in seconds since the Unix epoch; a parameter so tests can sign at a fixed time.
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

/// Shared, cheaply cloned request state.
#[derive(Clone)]
pub struct ApiState {
    inner: Arc<Inner>,
}

struct Inner {
    region: String,
    domains: Vec<String>,
    root_access_key: String,
    root_secret_key: SecretKey,
    list_tokens: TokenKey,
    max_inflight: usize,
    usage_interval_secs: u64,
    clock: Clock,
    storage: Storage,
    principals: Principals,
    notify_targets: Vec<String>,
    metrics: Arc<Metrics>,
    audit: OnceLock<tokio::sync::mpsc::Sender<AccessRecord>>,
}

impl ApiState {
    /// State from the server configuration and a clock.
    #[must_use]
    pub fn new(config: &S3Config, clock: Clock, storage: Storage) -> Self {
        Self::with_limit(config, clock, config.max_inflight, storage)
    }

    /// As [`ApiState::new`] with an explicit in-flight limit; `0` sheds every request (used to test the 503 path).
    #[must_use]
    pub fn with_limit(
        config: &S3Config,
        clock: Clock,
        max_inflight: usize,
        storage: Storage,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                region: config.region.clone(),
                domains: config.domains.clone(),
                root_access_key: config.root_access_key.clone(),
                root_secret_key: config.root_secret_key.clone(),
                list_tokens: TokenKey::derive(&config.root_secret_key),
                max_inflight,
                usage_interval_secs: config.usage_interval_secs,
                clock,
                storage,
                principals: Principals::new(),
                notify_targets: config
                    .notify_targets
                    .iter()
                    .map(|target| target.id.clone())
                    .collect(),
                metrics: Arc::new(Metrics::new()),
                audit: OnceLock::new(),
            }),
        }
    }

    /// Whether a notification destination with this id exists: the database's own topic, or one the operator
    /// declared.
    pub(crate) fn declares_target(&self, id: &str) -> bool {
        id == tessari_s3_core::notification::DATABASE_TARGET
            || self
                .inner
                .notify_targets
                .iter()
                .any(|declared| declared == id)
    }

    /// The system clock.
    #[must_use]
    pub fn system_clock() -> Clock {
        Arc::new(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .ok()
                .and_then(|elapsed| i64::try_from(elapsed.as_secs()).ok())
                .unwrap_or(i64::MAX)
        })
    }

    /// Now, by this state's clock.
    #[must_use]
    pub fn now(&self) -> i64 {
        (self.inner.clock)()
    }

    /// Seconds between usage passes; a quota is checked only against a measurement this recent (twice over).
    #[must_use]
    pub fn usage_interval_secs(&self) -> u64 {
        self.inner.usage_interval_secs
    }

    /// The region requests must be signed for.
    #[must_use]
    pub fn region(&self) -> &str {
        &self.inner.region
    }

    /// The endpoint domains for virtual-hosted addressing.
    #[must_use]
    pub fn domains(&self) -> &[String] {
        &self.inner.domains
    }

    /// Requests served at once before `SlowDown`.
    #[must_use]
    pub fn max_inflight(&self) -> usize {
        self.inner.max_inflight
    }

    /// The key listing continuation tokens are authenticated with.
    #[must_use]
    pub fn list_tokens(&self) -> &TokenKey {
        &self.inner.list_tokens
    }

    /// What this node served, refused and shed.
    #[must_use]
    pub fn metrics(&self) -> &Arc<Metrics> {
        &self.inner.metrics
    }

    /// Starts the access log: requests from now on queue a record, up to `capacity` waiting, for the writer this
    /// answers to append. `None` when it was started already.
    #[must_use]
    pub fn audit_writer(&self, capacity: usize) -> Option<crate::AuditWriter> {
        let (sender, receiver) = tokio::sync::mpsc::channel(capacity);
        self.inner.audit.set(sender).ok()?;
        Some(crate::AuditWriter::new(
            receiver,
            self.inner.storage.audit(),
            Arc::clone(&self.inner.metrics),
        ))
    }

    /// Where requests queue their access-log records, when the access log is started.
    pub(crate) fn audit(&self) -> Option<&tokio::sync::mpsc::Sender<AccessRecord>> {
        self.inner.audit.get()
    }

    /// The storage services.
    #[must_use]
    pub fn storage(&self) -> &Storage {
        &self.inner.storage
    }

    /// Who signs with `access_key`, presenting session token `token` when the request carried one, and the secret
    /// to verify the signature with: the root credential from configuration, a user's key from the store through the
    /// principal cache. Temporary credentials stand only with their own session token and until they expire; any
    /// other credential refuses a token, since nothing could check it.
    ///
    /// # Errors
    /// `InvalidAccessKeyId` when no credential has that key or it or its user is disabled; `InvalidToken` for a
    /// token missing, wrong or presented with a long-term key; `ExpiredToken` for temporary credentials past their
    /// expiry; the store's failure, which serves nothing cached in its place.
    pub(crate) async fn credential_for(
        &self,
        access_key: &str,
        token: Option<&str>,
    ) -> Result<Credential> {
        let now = self.now();
        let invalid_token = || {
            Error::new(
                ErrorCode::InvalidToken,
                "the security token is not valid for this access key",
            )
        };
        let expired = || Error::new(ErrorCode::ExpiredToken, "the security token has expired");
        if access_key == self.inner.root_access_key {
            return match token {
                Some(_) => Err(invalid_token()),
                None => Ok(Credential {
                    principal: Principal::Root,
                    secret: self.inner.root_secret_key.clone(),
                }),
            };
        }
        let live = resolve_user_key(
            &self.inner.principals,
            self.inner.storage.users(),
            self.inner.storage.identities(),
            access_key,
            now,
        )
        .await?;
        let Some(live) = live else {
            // The store drops temporary credentials when they expire; their token still names when that was.
            return Err(match token.and_then(session_token_expiry) {
                Some(expiry) if expiry <= now => expired(),
                _ => Error::new(
                    ErrorCode::InvalidAccessKeyId,
                    "the access key does not exist",
                ),
            });
        };
        match (&live.session, token) {
            (Some(session), Some(presented))
                if session_token_matches(presented, &session.token_digest) => {}
            (Some(_), _) | (None, Some(_)) => return Err(invalid_token()),
            (None, None) => {}
        }
        let secret = live.secret.clone();
        let session = live.session.is_some();
        principal_of(live, now)
            .map(|principal| Credential { principal, secret })
            .ok_or_else(|| {
                if session {
                    expired()
                } else {
                    Error::new(
                        ErrorCode::InvalidAccessKeyId,
                        "the access key does not exist",
                    )
                }
            })
    }
}
