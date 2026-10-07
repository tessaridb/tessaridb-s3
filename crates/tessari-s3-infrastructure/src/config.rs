//! Configuration from the environment (12-factor): every variable read once, at startup, with required ones
//! failing loudly by name.

use std::net::SocketAddr;

use tessari_s3_constants::{
    DEFAULT_LISTEN, DEFAULT_MAX_INFLIGHT, DEFAULT_META_CONNECTIONS, DEFAULT_RECLAIM_GRACE_SECS,
    DEFAULT_RECLAIM_INTERVAL_SECS, DEFAULT_REGION, DEFAULT_SHUTDOWN_GRACE_SECS,
    DEFAULT_UPLOAD_MAX_AGE_SECS, MIN_SECRET_KEY_LEN,
};
use tessari_s3_types::SecretKey;

use crate::tessaridb::{MetaSettings, is_safe_name};
use crate::{Error, Result};

/// Everything the S3 server reads from its environment.
#[derive(Debug, Clone)]
pub struct S3Config {
    /// `TESSARIDB_S3_LISTEN` — the address the S3 API listens on.
    pub listen: SocketAddr,
    /// `TESSARIDB_S3_REGION` — the region requests must be signed for.
    pub region: String,
    /// `TESSARIDB_S3_DOMAINS` — comma-separated endpoint domains for virtual-hosted addressing
    /// (`<bucket>.<domain>`); empty means path-style only.
    pub domains: Vec<String>,
    /// `TESSARIDB_S3_ROOT_ACCESS_KEY` — the root credential's access key (required).
    pub root_access_key: String,
    /// `TESSARIDB_S3_ROOT_SECRET_KEY` — the root credential's secret (required, at least 16 bytes).
    pub root_secret_key: SecretKey,
    /// `TESSARIDB_S3_MAX_INFLIGHT` — requests served at once before `SlowDown`.
    pub max_inflight: usize,
    /// `TESSARIDB_S3_SHUTDOWN_GRACE_SECS` — how long in-flight requests get after SIGTERM.
    pub shutdown_grace_secs: u64,
    /// `TESSARIDB_S3_META_*` — the TessariDB node holding every bucket and object record.
    pub meta: MetaSettings,
    /// `TESSARIDB_S3_DATA_DIR` — where objects above the inline size are stored; unset, they are refused.
    pub data_dir: Option<std::path::PathBuf>,
    /// `TESSARIDB_S3_RECLAIM_GRACE_SECS` — how long a queued data file is kept before it may be removed.
    pub reclaim_grace_secs: u64,
    /// `TESSARIDB_S3_RECLAIM_INTERVAL_SECS` — time between reclamation passes (positive).
    pub reclaim_interval_secs: u64,
    /// `TESSARIDB_S3_UPLOAD_MAX_AGE_SECS` — how long a multipart upload may stay open before the reaper aborts it
    /// (positive; counted from its initiation, as S3's AbortIncompleteMultipartUpload counts).
    pub upload_max_age_secs: u64,
}

impl S3Config {
    /// Reads the process environment.
    ///
    /// # Errors
    /// [`Error::MissingConfig`] or [`Error::InvalidConfig`] naming the variable.
    pub fn from_env() -> Result<Self> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    /// Reads through `lookup`, so tests supply variables without touching the process environment.
    ///
    /// # Errors
    /// As [`S3Config::from_env`].
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let get = |key: &str| lookup(key).filter(|value| !value.trim().is_empty());
        let listen_text = get("TESSARIDB_S3_LISTEN").unwrap_or_else(|| DEFAULT_LISTEN.to_owned());
        let listen = listen_text.parse().map_err(|_| Error::InvalidConfig {
            key: "TESSARIDB_S3_LISTEN",
            reason: "not an IP address and port",
        })?;
        let region = get("TESSARIDB_S3_REGION").unwrap_or_else(|| DEFAULT_REGION.to_owned());
        let domains = get("TESSARIDB_S3_DOMAINS")
            .map(|list| {
                list.split(',')
                    .map(|d| d.trim().to_ascii_lowercase())
                    .filter(|d| !d.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        let root_access_key = get("TESSARIDB_S3_ROOT_ACCESS_KEY").ok_or(Error::MissingConfig {
            key: "TESSARIDB_S3_ROOT_ACCESS_KEY",
        })?;
        if root_access_key.contains('/') {
            return Err(Error::InvalidConfig {
                key: "TESSARIDB_S3_ROOT_ACCESS_KEY",
                reason: "an access key may not contain '/'",
            });
        }
        let secret = get("TESSARIDB_S3_ROOT_SECRET_KEY").ok_or(Error::MissingConfig {
            key: "TESSARIDB_S3_ROOT_SECRET_KEY",
        })?;
        if secret.len() < MIN_SECRET_KEY_LEN {
            return Err(Error::InvalidConfig {
                key: "TESSARIDB_S3_ROOT_SECRET_KEY",
                reason: "shorter than 16 bytes",
            });
        }
        let max_inflight = parse_positive(
            get("TESSARIDB_S3_MAX_INFLIGHT"),
            "TESSARIDB_S3_MAX_INFLIGHT",
        )?
        .unwrap_or(DEFAULT_MAX_INFLIGHT);
        let shutdown_grace_secs = match get("TESSARIDB_S3_SHUTDOWN_GRACE_SECS") {
            None => DEFAULT_SHUTDOWN_GRACE_SECS,
            Some(text) => text.trim().parse().map_err(|_| Error::InvalidConfig {
                key: "TESSARIDB_S3_SHUTDOWN_GRACE_SECS",
                reason: "not a whole number of seconds",
            })?,
        };
        let meta = meta_settings(&get)?;
        Ok(Self {
            meta,
            reclaim_grace_secs: match get("TESSARIDB_S3_RECLAIM_GRACE_SECS") {
                None => DEFAULT_RECLAIM_GRACE_SECS,
                Some(text) => text.trim().parse().map_err(|_| Error::InvalidConfig {
                    key: "TESSARIDB_S3_RECLAIM_GRACE_SECS",
                    reason: "not a whole number of seconds",
                })?,
            },
            reclaim_interval_secs: parse_positive(
                get("TESSARIDB_S3_RECLAIM_INTERVAL_SECS"),
                "TESSARIDB_S3_RECLAIM_INTERVAL_SECS",
            )?
            .map_or(Ok(DEFAULT_RECLAIM_INTERVAL_SECS), |secs| {
                u64::try_from(secs).map_err(|_| Error::InvalidConfig {
                    key: "TESSARIDB_S3_RECLAIM_INTERVAL_SECS",
                    reason: "too large",
                })
            })?,
            upload_max_age_secs: parse_positive(
                get("TESSARIDB_S3_UPLOAD_MAX_AGE_SECS"),
                "TESSARIDB_S3_UPLOAD_MAX_AGE_SECS",
            )?
            .map_or(Ok(DEFAULT_UPLOAD_MAX_AGE_SECS), |secs| {
                u64::try_from(secs).map_err(|_| Error::InvalidConfig {
                    key: "TESSARIDB_S3_UPLOAD_MAX_AGE_SECS",
                    reason: "too large",
                })
            })?,
            data_dir: get("TESSARIDB_S3_DATA_DIR").map(|dir| std::path::PathBuf::from(dir.trim())),
            listen,
            region,
            domains,
            root_access_key,
            root_secret_key: SecretKey::new(secret),
            max_inflight,
            shutdown_grace_secs,
        })
    }
}

/// `TESSARIDB_S3_META_ADDRESS`, `_USER`, `_PASSWORD` (required), `_NAMESPACE` (`s3`), `_DATABASE` (`meta`),
/// `_CA` (a PEM file; unset speaks in the clear) and `_MAX_CONNECTIONS` (32).
fn meta_settings(get: &impl Fn(&str) -> Option<String>) -> Result<MetaSettings> {
    let required = |key: &'static str| get(key).ok_or(Error::MissingConfig { key });
    let name = |key: &'static str, default: &str| {
        let value = get(key).unwrap_or_else(|| default.to_owned());
        if is_safe_name(&value) {
            Ok(value)
        } else {
            Err(Error::InvalidConfig {
                key,
                reason: "must match [a-z][a-z0-9_]*, at most 63 characters",
            })
        }
    };
    let trust_pem = match get("TESSARIDB_S3_META_CA") {
        Some(path) => Some(
            std::fs::read(path.trim()).map_err(|_| Error::InvalidConfig {
                key: "TESSARIDB_S3_META_CA",
                reason: "the file cannot be read",
            })?,
        ),
        None => None,
    };
    Ok(MetaSettings {
        address: required("TESSARIDB_S3_META_ADDRESS")?,
        user: required("TESSARIDB_S3_META_USER")?,
        password: SecretKey::new(required("TESSARIDB_S3_META_PASSWORD")?),
        namespace: name("TESSARIDB_S3_META_NAMESPACE", "s3")?,
        database: name("TESSARIDB_S3_META_DATABASE", "meta")?,
        trust_pem,
        max_connections: parse_positive(
            get("TESSARIDB_S3_META_MAX_CONNECTIONS"),
            "TESSARIDB_S3_META_MAX_CONNECTIONS",
        )?
        .unwrap_or(DEFAULT_META_CONNECTIONS),
    })
}

/// Parses an optional positive integer.
fn parse_positive(text: Option<String>, key: &'static str) -> Result<Option<usize>> {
    text.map(|text| {
        text.trim()
            .parse::<usize>()
            .ok()
            .filter(|value| *value > 0)
            .ok_or(Error::InvalidConfig {
                key,
                reason: "not a positive whole number",
            })
    })
    .transpose()
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
