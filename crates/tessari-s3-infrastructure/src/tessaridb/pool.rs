//! A pool of wire connections to the metadata store. One connection is one session, so the namespace and database
//! travel WITH every unit of work rather than being set once: a reconnected connection has forgotten its `USE`, and
//! reading another database is not an error anybody would see.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use std::time::Duration;

use tessari_s3_constants::{META_DIAL_TIMEOUT_SECS, META_SIGN_IN_CONCURRENCY};
use tessaridb_client::{Answer, Client, Secured, Tls, Value};
use tokio::net::TcpStream;
use tokio::sync::Semaphore;

use super::error::{MetaError, MetaResult};
use super::settings::{MetaSettings, Replication, regional_database};

#[path = "vault.rs"]
mod vault;
pub use vault::VaultLink;

/// A cheaply cloned handle to the pool. Handles made by [`MetaPool::regional`] share the connections and differ only
/// in the database their units of work select.
#[derive(Clone)]
pub struct MetaPool {
    inner: Arc<Inner>,
    /// The database this handle's units of work select.
    database: Arc<str>,
    /// The `USE` sent ahead of every unit of work through this handle.
    scope: Arc<str>,
    /// The region whose database this handle selects; `None` on the global handle.
    region: Option<Arc<str>>,
}

struct Inner {
    settings: MetaSettings,
    tls: Option<Tls>,
    idle: Mutex<Vec<Link>>,
    permits: Semaphore,
    /// New connections signing in at once, held from the dial through the statement that spends the credential.
    signing_in: Semaphore,
    /// The address that answered last; the next dial starts there.
    preferred: AtomicUsize,
}

/// One open connection whose credential is already spent. The TLS client is boxed: it is ten times the size of the
/// plain one, and idle links sit in a vector.
enum Link {
    Plain(Client<TcpStream>),
    Secured(Box<Client<Secured>>),
}

impl Link {
    async fn run(
        &mut self,
        script: &str,
        credentials: Option<(&str, &str)>,
        parameters: Vec<(String, Value)>,
    ) -> tessaridb_client::Result<Vec<Answer>> {
        match self {
            Self::Plain(client) => client.run_with(script, credentials, parameters).await,
            Self::Secured(client) => client.run_with(script, credentials, parameters).await,
        }
    }
}

impl MetaPool {
    /// A pool for `settings`. Nothing is dialled until the first unit of work.
    ///
    /// # Errors
    /// [`MetaError::Configuration`] when the trust anchors cannot be read.
    pub fn new(settings: MetaSettings) -> MetaResult<Self> {
        let tls = match &settings.trust_pem {
            Some(pem) => Some(
                Tls::trusting_pem(pem).map_err(|_| MetaError::Configuration {
                    reason: "the metadata store's certificate authority is not readable PEM",
                })?,
            ),
            None => None,
        };
        let scope = use_statement(&settings.namespace, &settings.database);
        let database = Arc::from(settings.database.as_str());
        let permits = Semaphore::new(settings.max_connections);
        Ok(Self {
            inner: Arc::new(Inner {
                settings,
                tls,
                idle: Mutex::new(Vec::new()),
                permits,
                signing_in: Semaphore::new(META_SIGN_IN_CONCURRENCY),
                preferred: AtomicUsize::new(0),
            }),
            database,
            scope: Arc::from(scope),
            region: None,
        })
    }

    /// Runs `script` in the configured namespace and database, with `parameters` bound. One answer per statement of
    /// `script`; the `USE` that scopes it is not among them.
    ///
    /// # Errors
    /// [`MetaError::Refused`] with the node's own words and class, or [`MetaError::Unavailable`].
    pub async fn run(
        &self,
        script: &str,
        parameters: Vec<(String, Value)>,
    ) -> MetaResult<Vec<Answer>> {
        let scoped = format!("{}{script}", self.scope);
        let mut answers = self.run_raw(&scoped, parameters).await?;
        if answers.is_empty() {
            return Err(MetaError::Protocol {
                reason: "the scoping USE was not answered",
            });
        }
        answers.remove(0);
        Ok(answers)
    }

    /// Runs `script` exactly as written, for the schema script that creates the namespace and database.
    ///
    /// # Errors
    /// As [`MetaPool::run`].
    pub async fn run_unscoped(&self, script: &str) -> MetaResult<Vec<Answer>> {
        self.run_raw(script, Vec::new()).await
    }

    /// The configured namespace and the database this handle selects, for statements that must name them.
    #[must_use]
    pub fn scope(&self) -> (&str, &str) {
        (&self.inner.settings.namespace, &self.database)
    }

    /// A handle on the same connections whose units of work select the database holding `region`'s object
    /// metadata.
    ///
    /// # Errors
    /// [`MetaError::Configuration`] when `region` is not a region name, or the database name it makes is too long.
    pub fn regional(&self, region: &str) -> MetaResult<Self> {
        let settings = &self.inner.settings;
        let database =
            regional_database(&settings.database, region).ok_or(MetaError::Configuration {
                reason: "the region does not make a database name: lowercase letters, digits and single hyphens, \
                         beginning with a letter, and at most 63 characters with the database name",
            })?;
        Ok(Self {
            inner: Arc::clone(&self.inner),
            scope: Arc::from(use_statement(&settings.namespace, &database)),
            database: Arc::from(database),
            region: Some(Arc::from(region)),
        })
    }

    /// The region whose database this handle selects; `None` on the global handle.
    #[must_use]
    pub fn region(&self) -> Option<&str> {
        self.region.as_deref()
    }

    /// The database holding the records every region shares — buckets and identities.
    #[must_use]
    pub fn global_database(&self) -> &str {
        &self.inner.settings.database
    }

    /// How many copies the metadata cluster keeps of the namespace.
    #[must_use]
    pub fn replication(&self) -> Replication {
        self.inner.settings.replication
    }

    async fn run_raw(
        &self,
        script: &str,
        parameters: Vec<(String, Value)>,
    ) -> MetaResult<Vec<Answer>> {
        let _permit = self.inner.permits.acquire().await.map_err(|_| closed())?;
        let (mut link, signing) = match self.take_idle()? {
            Some(link) => (link, None),
            None => {
                let signing = self
                    .inner
                    .signing_in
                    .acquire()
                    .await
                    .map_err(|_| closed())?;
                // A connection may have come back while this request waited its turn to sign in.
                match self.take_idle()? {
                    Some(link) => (link, None),
                    None => (self.dial().await?, Some(signing)),
                }
            }
        };
        let credentials = signing.as_ref().map(|_| self.credentials());
        let credentials = credentials
            .as_ref()
            .map(|(user, password)| (user.as_str(), password.as_str()));
        let deadline = self.inner.settings.statement_timeout;
        let Ok(answered) =
            tokio::time::timeout(deadline, link.run(script, credentials, parameters)).await
        else {
            // Sent and not answered: the outcome is unknown and the connection's state with it, so it is dropped.
            return Err(MetaError::Unavailable {
                reason: format!(
                    "the metadata store did not answer within {} s",
                    deadline.as_secs()
                ),
            });
        };
        match answered {
            Ok(answers) => {
                self.put_idle(link)?;
                Ok(answers)
            }
            Err(tessaridb_client::Error::Refused { message, class }) => {
                // A refusal is the node's answer and the connection is healthy — unless the refused statement carried
                // the credential, when the refusal may be of the sign-in itself: a connection that never signed in is
                // dropped rather than handed to the next request as if it had.
                if signing.is_none() {
                    self.put_idle(link)?;
                }
                Err(MetaError::Refused { message, class })
            }
            // Anything else leaves the connection's state unknown, so it is dropped rather than reused.
            Err(error) => Err(MetaError::Unavailable {
                reason: error.to_string(),
            }),
        }
    }

    fn credentials(&self) -> (String, String) {
        (
            self.inner.settings.user.clone(),
            self.inner.settings.password.expose().to_owned(),
        )
    }

    /// Opens a connection to the first metadata node that answers, starting from the one that answered last, so a
    /// node that is down costs one failed dial per new connection rather than every request. A node that answers
    /// but does not lead is not skipped here: the client follows its redirect.
    async fn dial(&self) -> MetaResult<Link> {
        let addresses = &self.inner.settings.addresses;
        let start = self.inner.preferred.load(Ordering::Relaxed);
        let mut last = None;
        for index in dial_order(start, addresses.len()) {
            let Some(address) = addresses.get(index) else {
                continue;
            };
            match self.dial_one(address).await {
                Ok(link) => {
                    self.inner.preferred.store(index, Ordering::Relaxed);
                    return Ok(link);
                }
                Err(error) => {
                    tracing::warn!(address = %address, error = %error, "metadata node did not answer");
                    last = Some(error);
                }
            }
        }
        Err(last.unwrap_or(MetaError::Unavailable {
            reason: "no metadata node is configured".to_owned(),
        }))
    }

    async fn dial_one(&self, address: &str) -> MetaResult<Link> {
        let unavailable = |error: tessaridb_client::Error| MetaError::Unavailable {
            reason: error.to_string(),
        };
        let dialled = async {
            match &self.inner.tls {
                Some(tls) => Ok(Link::Secured(Box::new(
                    Client::connect_tls(address, tls)
                        .await
                        .map_err(unavailable)?,
                ))),
                None => Ok(Link::Plain(
                    Client::connect(address).await.map_err(unavailable)?,
                )),
            }
        };
        // A node behind an address that drops packets would otherwise hold this request for the OS connect timeout.
        tokio::time::timeout(Duration::from_secs(META_DIAL_TIMEOUT_SECS), dialled)
            .await
            .map_err(|_| MetaError::Unavailable {
                reason: format!("{address} did not answer within {META_DIAL_TIMEOUT_SECS} s"),
            })?
    }

    fn take_idle(&self) -> MetaResult<Option<Link>> {
        let mut idle = self.inner.idle.lock().map_err(|_| MetaError::Poisoned)?;
        Ok(idle.pop())
    }

    fn put_idle(&self, link: Link) -> MetaResult<()> {
        let mut idle = self.inner.idle.lock().map_err(|_| MetaError::Poisoned)?;
        idle.push(link);
        Ok(())
    }
}

/// The statement selecting `namespace` and `database`, both names already checked by [`super::is_safe_name`].
fn use_statement(namespace: &str, database: &str) -> String {
    format!("USE NAMESPACE {namespace} DATABASE {database};\n")
}

/// The refusal for a request that arrives after the pool closed.
fn closed() -> MetaError {
    MetaError::Unavailable {
        reason: "the metadata pool is closed".to_owned(),
    }
}

/// The order addresses are tried in: from `start` (taken modulo `len`) round to the one before it.
fn dial_order(start: usize, len: usize) -> impl Iterator<Item = usize> {
    (0..len).filter_map(move |offset| {
        start
            .checked_rem(len)?
            .checked_add(offset)?
            .checked_rem(len)
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tessari_s3_types::SecretKey;

    use super::{MetaPool, dial_order};
    use crate::tessaridb::{MetaSettings, Replication};

    fn settings() -> MetaSettings {
        MetaSettings {
            addresses: vec!["127.0.0.1:1".to_owned()],
            replication: Replication::None,
            user: "owner".to_owned(),
            password: SecretKey::new(String::new()),
            namespace: "s3".to_owned(),
            database: "meta".to_owned(),
            trust_pem: None,
            max_connections: 1,
            statement_timeout: Duration::from_secs(1),
        }
    }

    #[test]
    fn a_regional_handle_selects_its_regions_database_and_names_the_global_one() {
        let pool = MetaPool::new(settings()).expect("a pool over valid settings");
        let eu = pool.regional("eu-west-1").expect("a region name");
        assert_eq!(eu.scope(), ("s3", "meta_eu_west_1"));
        assert_eq!(eu.global_database(), "meta");
        assert_eq!(eu.region(), Some("eu-west-1"));
        assert_eq!(pool.region(), None);
        assert_eq!(
            pool.scope(),
            ("s3", "meta"),
            "the global handle is unchanged"
        );
        assert_eq!(pool.global_database(), "meta");
        assert!(matches!(
            pool.regional("EU"),
            Err(crate::tessaridb::MetaError::Configuration { .. })
        ));
    }

    #[test]
    fn addresses_are_tried_from_the_last_one_that_answered_round_to_the_rest() {
        let order = |start, len| dial_order(start, len).collect::<Vec<_>>();
        assert_eq!(order(2, 3), [2, 0, 1]);
        assert_eq!(order(5, 3), [2, 0, 1], "a stale start wraps");
        assert_eq!(order(0, 1), [0]);
        assert!(order(4, 0).is_empty());
    }
}
