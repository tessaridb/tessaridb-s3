//! The console's own key for `private_key_jwt` (RFC 7523): at the token endpoint it signs a short assertion instead of
//! sending a shared secret, so nothing that could be replayed for long crosses the wire, and a provider that issues no
//! client secrets can still authenticate the console.
//!
//! The key is Ed25519 (EdDSA). Its id is the RFC 7638 thumbprint of the public key, so the provider and the console
//! name it the same way without agreeing on anything else.

use std::fmt;
use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use openidconnect::core::{CoreEdDsaPrivateSigningKey, CoreJwsSigningAlgorithm};
use openidconnect::{CsrfToken, JsonWebKeyId, PrivateSigningKey};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tessari_s3_constants::CONSOLE_OIDC_ASSERTION_SECS;

/// The console's Ed25519 client key; immutable once loaded, so clones share it through an `Arc`.
#[derive(Clone)]
pub struct ClientKey {
    signing: Arc<CoreEdDsaPrivateSigningKey>,
    kid: String,
    public_jwk: String,
}

impl fmt::Debug for ClientKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClientKey")
            .field("kid", &self.kid)
            .finish_non_exhaustive()
    }
}

#[derive(Serialize)]
struct Header<'a> {
    alg: &'static str,
    typ: &'static str,
    kid: &'a str,
}

#[derive(Serialize)]
struct Claims<'a> {
    iss: &'a str,
    sub: &'a str,
    aud: &'a str,
    iat: i64,
    exp: i64,
    jti: &'a str,
}

#[derive(Serialize)]
struct PublicJwk<'a> {
    kty: &'static str,
    crv: &'static str,
    x: &'a str,
    kid: &'a str,
    alg: &'static str,
    #[serde(rename = "use")]
    use_: &'static str,
}

const NOT_ED25519: &str = "not an Ed25519 private key in PKCS#8 PEM";

impl ClientKey {
    /// The key a PKCS#8 PEM holds.
    ///
    /// # Errors
    /// A reason when `pem` is not an Ed25519 private key.
    pub fn from_pem(pem: &str) -> Result<Self, &'static str> {
        let unnamed =
            CoreEdDsaPrivateSigningKey::from_ed25519_pem(pem, None).map_err(|_| NOT_ED25519)?;
        let x = public_x(&unnamed)?;
        // RFC 7638 §3.2: the required members in lexicographic order, no whitespace.
        let canonical = format!(r#"{{"crv":"Ed25519","kty":"OKP","x":"{x}"}}"#);
        let kid = URL_SAFE_NO_PAD.encode(Sha256::digest(canonical.as_bytes()));
        let public_jwk = serde_json::to_string(&PublicJwk {
            kty: "OKP",
            crv: "Ed25519",
            x: &x,
            kid: &kid,
            alg: "EdDSA",
            use_: "sig",
        })
        .map_err(|_| "the public key could not be written as a JWK")?;
        let signing =
            CoreEdDsaPrivateSigningKey::from_ed25519_pem(pem, Some(JsonWebKeyId::new(kid.clone())))
                .map_err(|_| NOT_ED25519)?;
        Ok(Self {
            signing: Arc::new(signing),
            kid,
            public_jwk,
        })
    }

    /// The key's id: the RFC 7638 thumbprint of its public half.
    #[must_use]
    pub fn kid(&self) -> &str {
        &self.kid
    }

    /// The public half as a JWK, for registering the console at the provider; it holds nothing private.
    #[must_use]
    pub fn public_jwk(&self) -> String {
        self.public_jwk.clone()
    }

    /// A client assertion for `client_id`, addressed to `audience` (the provider's issuer), issued at `now` (seconds
    /// since the epoch) and valid for [`CONSOLE_OIDC_ASSERTION_SECS`], with an id of its own.
    ///
    /// # Errors
    /// A reason when the expiry overflows or the key cannot sign.
    pub fn assertion(
        &self,
        client_id: &str,
        audience: &str,
        now: i64,
    ) -> Result<String, &'static str> {
        let exp = now
            .checked_add(CONSOLE_OIDC_ASSERTION_SECS)
            .ok_or("the assertion's expiry overflows")?;
        let jti = CsrfToken::new_random();
        let header = serde_json::to_vec(&Header {
            alg: "EdDSA",
            typ: "JWT",
            kid: &self.kid,
        })
        .map_err(|_| "the assertion header could not be written")?;
        let claims = serde_json::to_vec(&Claims {
            iss: client_id,
            sub: client_id,
            aud: audience,
            iat: now,
            exp,
            jti: jti.secret(),
        })
        .map_err(|_| "the assertion claims could not be written")?;
        let signed = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(header),
            URL_SAFE_NO_PAD.encode(claims)
        );
        let signature = self
            .signing
            .sign(&CoreJwsSigningAlgorithm::EdDsa, signed.as_bytes())
            .map_err(|_| "the client key could not sign")?;
        Ok(format!("{signed}.{}", URL_SAFE_NO_PAD.encode(signature)))
    }
}

/// The base64url `x` of `key`'s public half.
fn public_x(key: &CoreEdDsaPrivateSigningKey) -> Result<String, &'static str> {
    // Read once at start from the library's own JWK type, whose fields are not public; not a per-request path.
    let public = serde_json::to_value(key.as_verification_key())
        .map_err(|_| "the public key could not be read")?;
    public
        .get("x")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or("the public key has no x")
}

#[cfg(test)]
#[path = "oidc_client_key_tests.rs"]
mod tests;
