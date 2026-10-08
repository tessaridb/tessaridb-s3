//! The cluster half of the configuration: read together or not at all, because a node with some of it would join a
//! cluster it cannot serve.

use tessari_s3_constants::{CLUSTER_SECRET_MIN_LEN, DEFAULT_HEAL_INTERVAL_SECS};
use tessari_s3_types::{Code, NodeId, SecretKey};

use crate::config::{ClusterSettings, parse_positive};
use crate::{Error, Result};

/// Every variable of the cluster group but the code itself.
const COMPANIONS: [&str; 5] = [
    "TESSARIDB_S3_NODE_ID",
    "TESSARIDB_S3_INTERNAL_LISTEN",
    "TESSARIDB_S3_INTERNAL_ADVERTISE",
    "TESSARIDB_S3_CLUSTER_SECRET",
    "TESSARIDB_S3_DATA_DIR",
];

/// `TESSARIDB_S3_ERASURE` with `_NODE_ID`, `_INTERNAL_LISTEN`, `_INTERNAL_ADVERTISE`, `_CLUSTER_SECRET` and
/// `TESSARIDB_S3_DATA_DIR`; no code means a node on its own, which may still have a data directory.
pub(crate) fn cluster_settings(
    get: &impl Fn(&str) -> Option<String>,
) -> Result<Option<ClusterSettings>> {
    let Some(code) = get("TESSARIDB_S3_ERASURE") else {
        let stray = COMPANIONS.iter().take(4).any(|key| get(key).is_some());
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
    }))
}

#[cfg(test)]
#[path = "cluster_config_tests.rs"]
mod tests;
