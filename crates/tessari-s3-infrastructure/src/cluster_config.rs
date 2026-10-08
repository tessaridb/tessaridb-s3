//! The cluster half of the configuration: read together or not at all, because a node with some of it would join a
//! cluster it cannot serve.

use tessari_s3_constants::{CLUSTER_SECRET_MIN_LEN, DEFAULT_HEAL_INTERVAL_SECS};
use tessari_s3_types::{Code, NodeId, SecretKey};

use crate::config::{ClusterSettings, parse_positive};
use crate::tls::InternalTls;
use crate::{Error, Result};

/// Every variable of the cluster group but the code itself.
const COMPANIONS: [&str; 5] = [
    "TESSARIDB_S3_NODE_ID",
    "TESSARIDB_S3_INTERNAL_LISTEN",
    "TESSARIDB_S3_INTERNAL_ADVERTISE",
    "TESSARIDB_S3_CLUSTER_SECRET",
    "TESSARIDB_S3_DATA_DIR",
];

/// The internal surface's TLS files: certificate chain, private key, cluster authority.
const TLS: [&str; 3] = [
    "TESSARIDB_S3_INTERNAL_TLS_CERT",
    "TESSARIDB_S3_INTERNAL_TLS_KEY",
    "TESSARIDB_S3_INTERNAL_TLS_CA",
];

/// The three TLS files read, all of them or none: a node told some of them would otherwise serve in the clear
/// while its operator believes it does not.
fn internal_tls(get: &impl Fn(&str) -> Option<String>) -> Result<Option<InternalTls>> {
    if TLS.iter().all(|key| get(key).is_none()) {
        return Ok(None);
    }
    let mut read = Vec::with_capacity(TLS.len());
    for key in TLS {
        let path = get(key).ok_or(Error::MissingConfig { key })?;
        read.push(
            std::fs::read(path.trim()).map_err(|_| Error::InvalidConfig {
                key,
                reason: "not a readable file",
            })?,
        );
    }
    let mut files = read.into_iter();
    match (files.next(), files.next(), files.next()) {
        (Some(certificate), Some(key), Some(authority)) => {
            Ok(Some(InternalTls::new(certificate, key, authority)))
        }
        _ => Err(Error::MissingConfig { key: TLS[0] }),
    }
}

/// `TESSARIDB_S3_ERASURE` with `_NODE_ID`, `_INTERNAL_LISTEN`, `_INTERNAL_ADVERTISE`, `_CLUSTER_SECRET` and
/// `TESSARIDB_S3_DATA_DIR`; no code means a node on its own, which may still have a data directory.
pub(crate) fn cluster_settings(
    get: &impl Fn(&str) -> Option<String>,
) -> Result<Option<ClusterSettings>> {
    let Some(code) = get("TESSARIDB_S3_ERASURE") else {
        let stray = COMPANIONS
            .iter()
            .take(4)
            .chain(TLS.iter())
            .any(|key| get(key).is_some());
        return if stray {
            Err(Error::InvalidConfig {
                key: "TESSARIDB_S3_ERASURE",
                reason: "cluster settings are set without the erasure code they belong to",
            })
        } else {
            Ok(None)
        };
    };
    let invalid = |key: &'static str, reason: &'static str| Error::InvalidConfig { key, reason };
    let code = Code::parse(code.trim()).map_err(|_| {
        invalid(
            "TESSARIDB_S3_ERASURE",
            "not k+m with k and m at least 1 and k+m at most 16",
        )
    })?;
    for key in COMPANIONS {
        if get(key).is_none() {
            return Err(Error::MissingConfig { key });
        }
    }
    let required = |key: &'static str| {
        get(key)
            .map(|value| value.trim().to_owned())
            .ok_or(Error::MissingConfig { key })
    };
    let node = NodeId::new(&required("TESSARIDB_S3_NODE_ID")?).map_err(|_| {
        invalid(
            "TESSARIDB_S3_NODE_ID",
            "1 to 63 lowercase letters, digits and inner hyphens",
        )
    })?;
    let internal_listen = required("TESSARIDB_S3_INTERNAL_LISTEN")?
        .parse()
        .map_err(|_| invalid("TESSARIDB_S3_INTERNAL_LISTEN", "not an IP address and port"))?;
    let internal_advertise = required("TESSARIDB_S3_INTERNAL_ADVERTISE")?;
    let secret = required("TESSARIDB_S3_CLUSTER_SECRET")?;
    if secret.len() < CLUSTER_SECRET_MIN_LEN {
        return Err(invalid(
            "TESSARIDB_S3_CLUSTER_SECRET",
            "shorter than 32 bytes",
        ));
    }
    let heal_interval_secs = parse_positive(
        get("TESSARIDB_S3_HEAL_INTERVAL_SECS"),
        "TESSARIDB_S3_HEAL_INTERVAL_SECS",
    )?
    .map_or(Ok(DEFAULT_HEAL_INTERVAL_SECS), |secs| {
        u64::try_from(secs).map_err(|_| invalid("TESSARIDB_S3_HEAL_INTERVAL_SECS", "too large"))
    })?;
    Ok(Some(ClusterSettings {
        node,
        code,
        internal_listen,
        internal_advertise,
        secret: SecretKey::new(secret),
        heal_interval_secs,
        tls: internal_tls(get)?,
    }))
}

#[cfg(test)]
#[path = "cluster_config_tests.rs"]
mod tests;
