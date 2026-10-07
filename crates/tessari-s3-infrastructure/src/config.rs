//! Configuration from the environment (12-factor): every variable read once, at startup, with required ones
//! failing loudly by name.

use std::net::SocketAddr;

use tessari_s3_constants::{
    DEFAULT_LISTEN, DEFAULT_MAX_INFLIGHT, DEFAULT_META_CONNECTIONS, DEFAULT_REGION,
    DEFAULT_SHUTDOWN_GRACE_SECS, MIN_SECRET_KEY_LEN,
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
mod tests {
    use std::collections::HashMap;

    use super::S3Config;
    use crate::Error;

    fn load(vars: &[(&str, &str)]) -> crate::Result<S3Config> {
        let map: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        S3Config::from_lookup(|key| map.get(key).cloned())
    }

    const CREDENTIALS: [(&str, &str); 5] = [
        ("TESSARIDB_S3_ROOT_ACCESS_KEY", "AKIAIOSFODNN7EXAMPLE"),
        (
            "TESSARIDB_S3_ROOT_SECRET_KEY",
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        ),
        ("TESSARIDB_S3_META_ADDRESS", "127.0.0.1:9080"),
        ("TESSARIDB_S3_META_USER", "s3"),
        ("TESSARIDB_S3_META_PASSWORD", "meta-password-0123"),
    ];

    #[test]
    fn defaults_apply_when_only_the_credentials_are_set() {
        let config = load(&CREDENTIALS).expect("loads");
        assert_eq!(config.listen.to_string(), "127.0.0.1:9100");
        assert_eq!(config.region, "us-east-1");
        assert!(config.domains.is_empty());
        assert_eq!(
            (config.max_inflight, config.shutdown_grace_secs),
            (1024, 30)
        );
        assert_eq!(
            (
                config.meta.namespace.as_str(),
                config.meta.database.as_str()
            ),
            ("s3", "meta")
        );
        assert_eq!(config.meta.max_connections, 32);
        assert!(config.meta.trust_pem.is_none());
    }

    #[test]
    fn a_tenancy_name_that_is_not_an_identifier_is_refused() {
        let mut vars = CREDENTIALS.to_vec();
        vars.push(("TESSARIDB_S3_META_NAMESPACE", "s3; DROP"));
        let refusal = load(&vars).map(|_| ()).expect_err("refused");
        assert!(
            matches!(
                refusal,
                Error::InvalidConfig {
                    key: "TESSARIDB_S3_META_NAMESPACE",
                    ..
                }
            ),
            "{refusal:?}"
        );
    }

    #[test]
    fn domains_are_split_trimmed_and_lowercased() {
        let mut vars = CREDENTIALS.to_vec();
        vars.push(("TESSARIDB_S3_DOMAINS", " S3.Example.com, ,s3.local"));
        assert_eq!(
            load(&vars).expect("loads").domains,
            ["s3.example.com", "s3.local"]
        );
    }

    #[test]
    fn missing_credentials_are_refused_by_name() {
        assert_eq!(
            load(&[]).map(|_| ()),
            Err(Error::MissingConfig {
                key: "TESSARIDB_S3_ROOT_ACCESS_KEY"
            })
        );
        let only_key = [CREDENTIALS[0]];
        assert_eq!(
            load(&only_key).map(|_| ()),
            Err(Error::MissingConfig {
                key: "TESSARIDB_S3_ROOT_SECRET_KEY"
            })
        );
    }

    #[test]
    fn invalid_values_are_refused_without_echoing_them() {
        let cases = [
            ("TESSARIDB_S3_ROOT_SECRET_KEY", "tiny-k3y"),
            ("TESSARIDB_S3_MAX_INFLIGHT", "0"),
            ("TESSARIDB_S3_LISTEN", "nowhere"),
            ("TESSARIDB_S3_SHUTDOWN_GRACE_SECS", "-1"),
        ];
        for (key, value) in cases {
            let mut vars = CREDENTIALS.to_vec();
            vars.retain(|(k, _)| *k != key);
            vars.push((key, value));
            let refusal = load(&vars).map(|_| ()).expect_err(key);
            assert!(
                matches!(refusal, Error::InvalidConfig { key: k, .. } if k == key),
                "{refusal:?}"
            );
            assert!(
                !refusal.to_string().contains(value),
                "the value is not echoed: {refusal}"
            );
        }
    }

    #[test]
    fn the_secret_never_appears_in_debug_output() {
        let config = load(&CREDENTIALS).expect("loads");
        let printed = format!("{config:?}");
        assert!(
            !printed.contains("EXAMPLEKEY") && !printed.contains("meta-password"),
            "{printed}"
        );
    }
}
