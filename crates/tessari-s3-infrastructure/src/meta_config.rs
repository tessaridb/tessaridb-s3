//! The metadata half of the configuration: the TessariDB node or nodes holding every record, and how this server
//! signs in and talks to them.

use std::num::NonZeroU8;
use std::time::Duration;

use tessari_s3_constants::{DEFAULT_META_CONNECTIONS, DEFAULT_META_STATEMENT_TIMEOUT_SECS};

use tessari_s3_types::SecretKey;

use crate::config::parse_positive;
use crate::tessaridb::{MetaSettings, Replication, is_safe_name};
use crate::{Error, Result};

/// `TESSARIDB_S3_META_ADDRESS` (one or more `host:port`, comma-separated), `_REPLICATION`, `_USER`, `_PASSWORD` (required), `_NAMESPACE` (`s3`), `_DATABASE` (`meta`),
/// `_CA` (a PEM file; unset speaks in the clear), `_MAX_CONNECTIONS` (32) and `_STATEMENT_TIMEOUT_SECS` (30).
pub(crate) fn meta_settings(get: &impl Fn(&str) -> Option<String>) -> Result<MetaSettings> {
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
    let addresses: Vec<String> = required("TESSARIDB_S3_META_ADDRESS")?
        .split(',')
        .map(str::trim)
        .filter(|address| !address.is_empty())
        .map(str::to_owned)
        .collect();
    if addresses.is_empty() {
        return Err(Error::MissingConfig {
            key: "TESSARIDB_S3_META_ADDRESS",
        });
    }
    let replication = replication(get("TESSARIDB_S3_META_REPLICATION"), addresses.len())?;
    Ok(MetaSettings {
        addresses,
        replication,
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
        statement_timeout: Duration::from_secs(statement_timeout_secs(get)?),
    })
}

/// `TESSARIDB_S3_META_STATEMENT_TIMEOUT_SECS`: a positive number of seconds.
fn statement_timeout_secs(get: &impl Fn(&str) -> Option<String>) -> Result<u64> {
    const KEY: &str = "TESSARIDB_S3_META_STATEMENT_TIMEOUT_SECS";
    match parse_positive(get(KEY), KEY)? {
        Some(secs) => u64::try_from(secs).map_err(|_| Error::InvalidConfig {
            key: KEY,
            reason: "too large",
        }),
        None => Ok(DEFAULT_META_STATEMENT_TIMEOUT_SECS),
    }
}

/// `TESSARIDB_S3_META_REPLICATION`: `none` or a number of copies. Unset means `none` for one metadata node and is
/// refused for several, because a cluster refuses a namespace whose replication nobody stated and a guessed default
/// would decide how many copies exist.
fn replication(text: Option<String>, nodes: usize) -> Result<Replication> {
    const KEY: &str = "TESSARIDB_S3_META_REPLICATION";
    let Some(text) = text else {
        return if nodes > 1 {
            Err(Error::InvalidConfig {
                key: KEY,
                reason: "required when more than one metadata node is listed: a number of copies or none",
            })
        } else {
            Ok(Replication::None)
        };
    };
    let text = text.trim();
    if text.eq_ignore_ascii_case("none") {
        return Ok(Replication::None);
    }
    text.parse::<NonZeroU8>()
        .map(Replication::Factor)
        .map_err(|_| Error::InvalidConfig {
            key: KEY,
            reason: "a number of copies from 1 to 255, or none",
        })
}
