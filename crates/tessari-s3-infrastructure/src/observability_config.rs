//! What a node tells about itself: whether it records every request in the access log, and where it serves its
//! metrics.

use std::net::SocketAddr;

use crate::{Error, Result};

/// `TESSARIDB_S3_AUDIT`: `on` (the default) or `off`.
pub(crate) fn audit(get: &impl Fn(&str) -> Option<String>) -> Result<bool> {
    match get("TESSARIDB_S3_AUDIT").as_deref().map(str::trim) {
        None | Some("on") => Ok(true),
        Some("off") => Ok(false),
        Some(_) => Err(Error::InvalidConfig {
            key: "TESSARIDB_S3_AUDIT",
            reason: "not on or off",
        }),
    }
}

/// `TESSARIDB_S3_METRICS_LISTEN`, or `None` when unset.
pub(crate) fn metrics_listen(get: &impl Fn(&str) -> Option<String>) -> Result<Option<SocketAddr>> {
    get("TESSARIDB_S3_METRICS_LISTEN")
        .map(|text| {
            text.trim().parse().map_err(|_| Error::InvalidConfig {
                key: "TESSARIDB_S3_METRICS_LISTEN",
                reason: "not an IP address and port",
            })
        })
        .transpose()
}

#[cfg(test)]
#[path = "observability_config_tests.rs"]
mod tests;
