//! The configuration this node runs on: the environment's bootstrap part reaches the metadata store, and the
//! settings stored there — written from the environment the first time a node starts with them — are read before
//! the rest of the configuration, then re-read while the node runs so a changed one applies without a restart.

use std::sync::Arc;
use std::time::Duration;

use tessari_s3_constants::{DAEMON_RUN_TIMEOUT_SECS, SETTINGS_REREAD_SECS};
use tessari_s3_daemons::{Daemon, DaemonFuture, Error, Report};
use tessari_s3_infrastructure::settings_keys::{Class, stored_setting};
use tessari_s3_infrastructure::settings_source::Resolved;
use tessari_s3_infrastructure::tessaridb::MetaPool;
use tessari_s3_infrastructure::{Bootstrap, LiveSettings, S3Config};
use tessari_s3_storage::keys::Passphrase;
use tessari_s3_storage::settings::SettingsService;
use zeroize::Zeroizing;

/// Where a variable is read from that the metadata store does not hold.
pub(crate) type Environment = fn(&str) -> Option<String>;

/// The configuration from `env` and the settings the metadata store holds for this node's region, and what re-reads
/// the store while the node runs.
///
/// A keys passphrase that is set must be a usable one: it opens the stored secrets, and a node that cannot open them
/// does not start on whatever its environment happens to hold instead.
///
/// # Errors
/// A variable missing or invalid, by name; the metadata store's refusal; a fixed setting the environment
/// contradicts, by name.
pub(crate) async fn configure(env: Environment) -> anyhow::Result<(S3Config, Reread)> {
    let bootstrap = Bootstrap::from_lookup(&env)?;
    let passphrase = match &bootstrap.keys_passphrase {
        Some(text) => Some(Passphrase::new(Zeroizing::new(text.expose().to_owned()))?),
        None => None,
    };
    let settings = SettingsService::new(MetaPool::new(bootstrap.meta)?);
    let resolved = settings
        .resolve(&bootstrap.region, &env, passphrase.as_ref())
        .await?;
    // Checked before anything is kept: a value this node refuses is never written for the others to start on.
    let config = S3Config::from_lookup(|key| resolved.lookup(&env, key))?;
    settings
        .keep(&bootstrap.region, &resolved, passphrase.as_ref())
        .await?;
    let reread = Reread {
        settings,
        region: bootstrap.region,
        started: resolved,
        env,
    };
    Ok((config, reread))
}

/// What the node started on, kept so the plain settings can be read again over it.
pub(crate) struct Reread {
    settings: SettingsService,
    region: String,
    started: Resolved,
    env: Environment,
}

impl Reread {
    /// The daemon that re-reads the store every [`SETTINGS_REREAD_SECS`] and publishes into `live`.
    pub(crate) fn daemon(self, live: Arc<LiveSettings>) -> Arc<dyn Daemon> {
        Arc::new(Refresh { reread: self, live })
    }
}

/// Re-reads the plain settings — never the secrets, which would be a recorded reveal every few seconds and none of
/// which applies live — checks the whole configuration again, and publishes the values that apply while running.
struct Refresh {
    reread: Reread,
    live: Arc<LiveSettings>,
}

const NAME: &str = "settings-refresh";

fn failed(error: &dyn std::fmt::Display) -> Error {
    Error::Run {
        daemon: NAME,
        message: error.to_string(),
    }
}

impl Daemon for Refresh {
    fn name(&self) -> &'static str {
        NAME
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(SETTINGS_REREAD_SECS)
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(DAEMON_RUN_TIMEOUT_SECS)
    }

    fn run_once(&self) -> DaemonFuture<'_> {
        Box::pin(async move {
            let reread = &self.reread;
            let held = reread
                .settings
                .settings(&reread.region)
                .await
                .map_err(|error| failed(&error))?;
            // A value that fails the configuration's checks is refused by name and the one running stays.
            let config = S3Config::from_lookup(|key| match stored_setting(key) {
                Some(stored) if stored.class == Class::Setting => held
                    .get(key)
                    .cloned()
                    .or_else(|| reread.started.lookup(&reread.env, key)),
                _ => reread.started.lookup(&reread.env, key),
            })
            .map_err(|error| failed(&error))?;
            let changed = self.live.publish(&config);
            Ok(Report {
                idle: changed.is_empty(),
                message: if changed.is_empty() {
                    "no setting changed".to_owned()
                } else {
                    format!("applied {}", changed.join(", "))
                },
            })
        })
    }
}
