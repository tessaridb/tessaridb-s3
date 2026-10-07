//! A pool of wire connections to the metadata store. One connection is one session, so the namespace and database
//! travel WITH every unit of work rather than being set once: a reconnected connection has forgotten its `USE`, and
//! reading another database is not an error anybody would see.

use std::sync::{Arc, Mutex};

use tessaridb_client::{Answer, Client, Secured, Tls, Value};
use tokio::net::TcpStream;
use tokio::sync::Semaphore;

use super::error::{MetaError, MetaResult};
use super::settings::MetaSettings;

/// A cheaply cloned handle to the pool.
#[derive(Clone)]
pub struct MetaPool {
    inner: Arc<Inner>,
}

struct Inner {
    settings: MetaSettings,
    tls: Option<Tls>,
    scope: String,
    idle: Mutex<Vec<Link>>,
    permits: Semaphore,
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
        let scope = format!(
            "USE NAMESPACE {} DATABASE {};\n",
            settings.namespace, settings.database
        );
        let permits = Semaphore::new(settings.max_connections);
        Ok(Self {
            inner: Arc::new(Inner {
                settings,
                tls,
                scope,
                idle: Mutex::new(Vec::new()),
                permits,
            }),
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
        let scoped = format!("{}{script}", self.inner.scope);
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

    /// The configured namespace and database, for statements that must name them.
    #[must_use]
    pub fn scope(&self) -> (&str, &str) {
        (
            &self.inner.settings.namespace,
            &self.inner.settings.database,
        )
    }

    async fn run_raw(
        &self,
        script: &str,
        parameters: Vec<(String, Value)>,
    ) -> MetaResult<Vec<Answer>> {
        let _permit = self
            .inner
            .permits
            .acquire()
            .await
            .map_err(|_| MetaError::Unavailable {
                reason: "the metadata pool is closed".to_owned(),
            })?;
        let pooled = self.take_idle()?;
        let (mut link, credentials) = match pooled {
            Some(link) => (link, None),
            None => (self.dial().await?, Some(self.credentials())),
        };
        let credentials = credentials
            .as_ref()
            .map(|(user, password)| (user.as_str(), password.as_str()));
        match link.run(script, credentials, parameters).await {
            Ok(answers) => {
                self.put_idle(link)?;
                Ok(answers)
            }
            Err(tessaridb_client::Error::Refused { message, class }) => {
                // A refusal is the node's answer; the connection is healthy and goes back.
                self.put_idle(link)?;
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

    async fn dial(&self) -> MetaResult<Link> {
        let address = &self.inner.settings.address;
        let unavailable = |error: tessaridb_client::Error| MetaError::Unavailable {
            reason: error.to_string(),
        };
        match &self.inner.tls {
            Some(tls) => Ok(Link::Secured(Box::new(
                Client::connect_tls(address, tls)
                    .await
                    .map_err(unavailable)?,
            ))),
            None => Ok(Link::Plain(
                Client::connect(address.as_str())
                    .await
                    .map_err(unavailable)?,
            )),
        }
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
