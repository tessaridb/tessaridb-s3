//! The console half of the configuration: where the operator console listens, whether it speaks TLS, and how fast
//! it lets a client sign in and call it.

use std::net::SocketAddr;

use tessari_s3_constants::{
    DEFAULT_CONSOLE_REQUESTS_PER_MINUTE, DEFAULT_CONSOLE_SIGN_INS_PER_MINUTE,
};

use crate::config::parse_positive;
use crate::tls::ServerTls;
use crate::{Error, Result};

/// Where and how the operator console is served. Absent means no console.
#[derive(Debug, Clone)]
pub struct ConsoleSettings {
    /// `TESSARIDB_S3_CONSOLE_LISTEN` — the address the console listens on.
    pub listen: SocketAddr,
    /// `TESSARIDB_S3_CONSOLE_TLS_CERT` and `_KEY` — both or neither; `None` serves in the clear.
    pub tls: Option<ServerTls>,
    /// `TESSARIDB_S3_CONSOLE_SIGN_INS_PER_MINUTE` — sign-in attempts per client address per minute.
    pub sign_ins_per_minute: usize,
    /// `TESSARIDB_S3_CONSOLE_REQUESTS_PER_MINUTE` — console requests per signed-in key per minute.
    pub requests_per_minute: usize,
    /// `TESSARIDB_S3_CONSOLE_OIDC_*` — the identity provider people sign in through; `None` keeps key sign-in only.
    pub oidc: Option<OidcSettings>,
}

/// The OpenID Connect provider the console signs people in through.
#[derive(Debug, Clone)]
pub struct OidcSettings {
    /// `TESSARIDB_S3_CONSOLE_OIDC_ISSUER` — the issuer URL; discovery and keys are read from it alone.
    pub issuer: String,
    /// `TESSARIDB_S3_CONSOLE_OIDC_CLIENT_ID` — the console's client id at the provider.
    pub client_id: String,
    /// `TESSARIDB_S3_CONSOLE_OIDC_CLIENT_SECRET` — its client secret (`client_secret_basic`).
    pub client_secret: tessari_s3_types::SecretKey,
    /// `TESSARIDB_S3_CONSOLE_OIDC_REDIRECT_URI` — the callback URL registered at the provider, exactly.
    pub redirect_uri: String,
    /// `TESSARIDB_S3_CONSOLE_OIDC_MFA_AMR` — `amr` values that prove a second factor; one must be present.
    pub mfa_amr: Vec<String>,
    /// `TESSARIDB_S3_CONSOLE_BREAK_GLASS_WEBHOOK` — where a root sign-in is announced as it happens, if anywhere.
    pub break_glass_webhook: Option<String>,
    /// `TESSARIDB_S3_CONSOLE_KEY_SIGN_IN` — which access keys still open a console session beside the provider.
    pub key_sign_in: KeySignIn,
}

/// Which access keys open a console session where an identity provider is configured. The S3 API is not affected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySignIn {
    /// `all` — a user's key and the root key, as without a provider.
    All,
    /// `root` — people sign in through the provider only; the root key remains, as the emergency path.
    Root,
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
            .chain(crate::oidc_config::ALL)
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
    let tls = tls_pair(get, CERT, KEY)?;
    Ok(Some(ConsoleSettings {
        listen,
        tls,
        sign_ins_per_minute: parse_positive(get(SIGN_INS), SIGN_INS)?
            .unwrap_or(DEFAULT_CONSOLE_SIGN_INS_PER_MINUTE),
        requests_per_minute: parse_positive(get(REQUESTS), REQUESTS)?
            .unwrap_or(DEFAULT_CONSOLE_REQUESTS_PER_MINUTE),
        oidc: crate::oidc_config::oidc_settings(get)?,
    }))
}

/// A listener's certificate and key, read from the files `cert` and `key` name: both or neither, and a half pair is
/// refused naming the missing one, since an operator who set one file believes the listener speaks TLS.
pub(crate) fn tls_pair(
    get: &impl Fn(&str) -> Option<String>,
    cert: &'static str,
    key: &'static str,
) -> Result<Option<ServerTls>> {
    match (get(cert), get(key)) {
        (None, None) => Ok(None),
        (Some(_), None) => Err(Error::MissingConfig { key }),
        (None, Some(_)) => Err(Error::MissingConfig { key: cert }),
        (Some(cert_path), Some(key_path)) => Ok(Some(ServerTls::new(
            read(cert, &cert_path)?,
            read(key, &key_path)?,
        ))),
    }
}

fn read(key: &'static str, path: &str) -> Result<Vec<u8>> {
    std::fs::read(path.trim()).map_err(|_| Error::InvalidConfig {
        key,
        reason: "not a readable file",
    })
}

#[cfg(test)]
#[path = "console_config_tests.rs"]
pub(crate) mod tests;
