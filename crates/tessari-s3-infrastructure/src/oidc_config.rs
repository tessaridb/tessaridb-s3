//! The identity provider the console signs people in through: all of it or none of it, every URL over https unless
//! it points at this machine, and a second factor always required.

use tessari_s3_constants::CONSOLE_OIDC_DEFAULT_MFA_AMR;
use tessari_s3_types::SecretKey;

use crate::console_config::{ClientAuth, KeySignIn, OidcSettings};
use crate::oidc_client_key::ClientKey;
use crate::{Error, Result};

pub(crate) const ISSUER: &str = "TESSARIDB_S3_CONSOLE_OIDC_ISSUER";
pub(crate) const CLIENT_ID: &str = "TESSARIDB_S3_CONSOLE_OIDC_CLIENT_ID";
pub(crate) const CLIENT_SECRET: &str = "TESSARIDB_S3_CONSOLE_OIDC_CLIENT_SECRET";
pub(crate) const CLIENT_KEY: &str = "TESSARIDB_S3_CONSOLE_OIDC_CLIENT_KEY";
pub(crate) const REDIRECT_URI: &str = "TESSARIDB_S3_CONSOLE_OIDC_REDIRECT_URI";
pub(crate) const MFA_AMR: &str = "TESSARIDB_S3_CONSOLE_OIDC_MFA_AMR";
pub(crate) const BREAK_GLASS_WEBHOOK: &str = "TESSARIDB_S3_CONSOLE_BREAK_GLASS_WEBHOOK";
pub(crate) const KEY_SIGN_IN: &str = "TESSARIDB_S3_CONSOLE_KEY_SIGN_IN";

/// Every variable of this group, the issuer first.
pub(crate) const ALL: [&str; 8] = [
    ISSUER,
    CLIENT_ID,
    CLIENT_SECRET,
    CLIENT_KEY,
    REDIRECT_URI,
    MFA_AMR,
    BREAK_GLASS_WEBHOOK,
    KEY_SIGN_IN,
];

/// The provider, when `TESSARIDB_S3_CONSOLE_OIDC_ISSUER` is set; a companion without it is refused.
pub(crate) fn oidc_settings(get: &impl Fn(&str) -> Option<String>) -> Result<Option<OidcSettings>> {
    let Some(issuer) = get(ISSUER) else {
        if let Some(key) = ALL.into_iter().skip(1).find(|key| get(key).is_some()) {
            return Err(Error::InvalidConfig {
                key,
                reason: "set without TESSARIDB_S3_CONSOLE_OIDC_ISSUER",
            });
        }
        return Ok(None);
    };
    let required = |key: &'static str| {
        get(key)
            .map(|value| value.trim().to_owned())
            .ok_or(Error::MissingConfig { key })
    };
    let mfa_amr: Vec<String> = get(MFA_AMR).map_or_else(
        || vec![CONSOLE_OIDC_DEFAULT_MFA_AMR.to_owned()],
        |list| {
            list.split(',')
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
                .collect()
        },
    );
    if mfa_amr.is_empty() {
        return Err(Error::InvalidConfig {
            key: MFA_AMR,
            reason: "no amr value: a second factor is always required",
        });
    }
    Ok(Some(OidcSettings {
        issuer: url(ISSUER, &issuer)?,
        client_id: required(CLIENT_ID)?,
        client_auth: client_auth(get)?,
        redirect_uri: url(REDIRECT_URI, &required(REDIRECT_URI)?)?,
        mfa_amr,
        break_glass_webhook: get(BREAK_GLASS_WEBHOOK)
            .map(|text| url(BREAK_GLASS_WEBHOOK, &text))
            .transpose()?,
        key_sign_in: match get(KEY_SIGN_IN).as_deref().map(str::trim) {
            None | Some("all") => KeySignIn::All,
            Some("root") => KeySignIn::Root,
            Some(_) => {
                return Err(Error::InvalidConfig {
                    key: KEY_SIGN_IN,
                    reason: "neither all nor root",
                });
            }
        },
    }))
}

/// The secret or the key file, exactly one of them; the key is read and parsed here, so a bad file stops the start.
fn client_auth(get: &impl Fn(&str) -> Option<String>) -> Result<ClientAuth> {
    match (get(CLIENT_SECRET), get(CLIENT_KEY)) {
        (Some(_), Some(_)) => Err(Error::InvalidConfig {
            key: CLIENT_KEY,
            reason: "set beside TESSARIDB_S3_CONSOLE_OIDC_CLIENT_SECRET: give one of the two",
        }),
        (Some(secret), None) => Ok(ClientAuth::Secret(SecretKey::new(secret.trim().to_owned()))),
        (None, Some(path)) => {
            let pem = std::fs::read_to_string(path.trim()).map_err(|_| Error::InvalidConfig {
                key: CLIENT_KEY,
                reason: "cannot be read",
            })?;
            ClientKey::from_pem(&pem)
                .map(ClientAuth::Key)
                .map_err(|reason| Error::InvalidConfig {
                    key: CLIENT_KEY,
                    reason,
                })
        }
        (None, None) => Err(Error::MissingConfig { key: CLIENT_SECRET }),
    }
}

/// `text` as a URL over https, or over http to this machine only.
fn url(key: &'static str, text: &str) -> Result<String> {
    let text = text.trim();
    let parsed = reqwest::Url::parse(text).map_err(|_| Error::InvalidConfig {
        key,
        reason: "not a URL",
    })?;
    let loopback = matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    match parsed.scheme() {
        "https" if parsed.host_str().is_some() => Ok(text.to_owned()),
        "http" if loopback => Ok(text.to_owned()),
        _ => Err(Error::InvalidConfig {
            key,
            reason: "not an https URL (http only to this machine)",
        }),
    }
}

#[cfg(test)]
#[path = "oidc_config_tests.rs"]
mod tests;
