//! The console half of the configuration: where the operator console listens, whether it speaks TLS, and how fast
//! it lets a client sign in and call it.

use std::net::SocketAddr;

use tessari_s3_constants::{
    DEFAULT_CONSOLE_REQUESTS_PER_MINUTE, DEFAULT_CONSOLE_SIGN_INS_PER_MINUTE,
};

use crate::config::parse_positive;
use crate::tls::ConsoleTls;
use crate::{Error, Result};

/// Where and how the operator console is served. Absent means no console.
#[derive(Debug, Clone)]
pub struct ConsoleSettings {
    /// `TESSARIDB_S3_CONSOLE_LISTEN` — the address the console listens on.
    pub listen: SocketAddr,
    /// `TESSARIDB_S3_CONSOLE_TLS_CERT` and `_KEY` — both or neither; `None` serves in the clear.
    pub tls: Option<ConsoleTls>,
    /// `TESSARIDB_S3_CONSOLE_SIGN_INS_PER_MINUTE` — sign-in attempts per client address per minute.
    pub sign_ins_per_minute: usize,
    /// `TESSARIDB_S3_CONSOLE_REQUESTS_PER_MINUTE` — console requests per signed-in key per minute.
    pub requests_per_minute: usize,
}

const LISTEN: &str = "TESSARIDB_S3_CONSOLE_LISTEN";
const CERT: &str = "TESSARIDB_S3_CONSOLE_TLS_CERT";
const KEY: &str = "TESSARIDB_S3_CONSOLE_TLS_KEY";
const SIGN_INS: &str = "TESSARIDB_S3_CONSOLE_SIGN_INS_PER_MINUTE";
const REQUESTS: &str = "TESSARIDB_S3_CONSOLE_REQUESTS_PER_MINUTE";

/// The console settings, when `TESSARIDB_S3_CONSOLE_LISTEN` is set. A companion variable without it is refused: an
/// operator who set a certificate believes the console is served over TLS.
pub(crate) fn console_settings(
    get: &impl Fn(&str) -> Option<String>,
) -> Result<Option<ConsoleSettings>> {
    let Some(listen) = get(LISTEN) else {
        if let Some(key) = [CERT, KEY, SIGN_INS, REQUESTS]
            .into_iter()
            .find(|key| get(key).is_some())
        {
            return Err(Error::InvalidConfig {
                key,
                reason: "set without TESSARIDB_S3_CONSOLE_LISTEN",
            });
        }
        return Ok(None);
    };
    let listen = listen.trim().parse().map_err(|_| Error::InvalidConfig {
        key: LISTEN,
        reason: "not an IP address and port",
    })?;
    let tls = match (get(CERT), get(KEY)) {
        (None, None) => None,
        (Some(_), None) => return Err(Error::MissingConfig { key: KEY }),
        (None, Some(_)) => return Err(Error::MissingConfig { key: CERT }),
        (Some(cert), Some(key)) => Some(ConsoleTls::new(read(CERT, &cert)?, read(KEY, &key)?)),
    };
    Ok(Some(ConsoleSettings {
        listen,
        tls,
        sign_ins_per_minute: parse_positive(get(SIGN_INS), SIGN_INS)?
            .unwrap_or(DEFAULT_CONSOLE_SIGN_INS_PER_MINUTE),
        requests_per_minute: parse_positive(get(REQUESTS), REQUESTS)?
            .unwrap_or(DEFAULT_CONSOLE_REQUESTS_PER_MINUTE),
    }))
}

fn read(key: &'static str, path: &str) -> Result<Vec<u8>> {
    std::fs::read(path.trim()).map_err(|_| Error::InvalidConfig {
        key,
        reason: "not a readable file",
    })
}

#[cfg(test)]
#[path = "console_config_tests.rs"]
mod tests;
