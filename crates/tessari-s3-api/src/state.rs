//! What every request handler shares: the signing region, the endpoint domains, the root credential and the clock.

use std::sync::Arc;

use tessari_s3_core::objects::token::TokenKey;
use tessari_s3_infrastructure::S3Config;
use tessari_s3_storage::Storage;
use tessari_s3_types::{ErrorCode, SecretKey};

use crate::{Error, Result};

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
    clock: Clock,
    storage: Storage,
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
                clock,
                storage,
            }),
        }
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

    /// The storage services.
    #[must_use]
    pub fn storage(&self) -> &Storage {
        &self.inner.storage
    }

    /// The secret for `access_key`.
    ///
    /// # Errors
    /// `InvalidAccessKeyId` when no credential has that key.
    pub fn secret_for(&self, access_key: &str) -> Result<&SecretKey> {
        if access_key == self.inner.root_access_key {
            Ok(&self.inner.root_secret_key)
        } else {
            Err(Error::new(
                ErrorCode::InvalidAccessKeyId,
                "the access key does not exist",
            ))
        }
    }
}
