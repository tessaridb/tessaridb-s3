//! SigV4 in the `Authorization` header.

use tessari_s3_constants::SIGV4_MAX_CLOCK_SKEW_SECS;

use tessari_s3_types::SecretKey;

use super::canonical::{SignedRequest, check_required_signed};
use super::payload::parse_payload_hash;
use super::signing::{
    ALGORITHM, CredentialScope, SignatureClaim, is_signature_hex, parse_credential,
    parse_signed_headers, verify_signature,
};
use super::time::AmzDateTime;
use super::verified::Verified;
use super::{AuthError, AuthResult};

/// A parsed `Authorization: AWS4-HMAC-SHA256 Credential=…, SignedHeaders=…, Signature=…` header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizationHeader {
    access_key: String,
    scope: CredentialScope,
    signed_headers: Vec<String>,
    signature: String,
}

impl AuthorizationHeader {
    /// Parses the header value.
    ///
    /// # Errors
    /// [`AuthError::UnsupportedAlgorithm`] or [`AuthError::Malformed`].
    pub fn parse(value: &str) -> AuthResult<Self> {
        let (algorithm, fields) = value
            .trim()
            .split_once(' ')
            .ok_or(AuthError::UnsupportedAlgorithm)?;
        if algorithm != ALGORITHM {
            return Err(AuthError::UnsupportedAlgorithm);
        }
        let (mut credential, mut signed_headers, mut signature) = (None, None, None);
        for field in fields.split(',').map(str::trim) {
            let (name, field_value) = field.split_once('=').ok_or(AuthError::Malformed {
                reason: "authorization",
            })?;
            let slot = match name {
                "Credential" => &mut credential,
                "SignedHeaders" => &mut signed_headers,
                "Signature" => &mut signature,
                _ => {
                    return Err(AuthError::Malformed {
                        reason: "authorization",
                    });
                }
            };
            if slot.replace(field_value).is_some() {
                return Err(AuthError::Malformed {
                    reason: "authorization field repeated",
                });
            }
        }
        let missing = AuthError::Malformed {
            reason: "authorization field missing",
        };
        let (access_key, scope) = parse_credential(credential.ok_or(missing.clone())?)?;
        let signed_headers = parse_signed_headers(signed_headers.ok_or(missing.clone())?)?;
        let signature = signature.ok_or(missing)?;
        if !is_signature_hex(signature) {
            return Err(AuthError::Malformed {
                reason: "signature",
            });
        }
        Ok(Self {
            access_key,
            scope,
            signed_headers,
            signature: signature.to_owned(),
        })
    }

    /// The access key: look its secret up, then call [`verify_header`].
    #[must_use]
    pub fn access_key(&self) -> &str {
        &self.access_key
    }
}

/// Verifies a header-signed request against the secret of its access key, the server's region and clock.
///
/// # Errors
/// Any [`AuthError`]; the request is then refused with [`AuthError::s3_code`].
pub fn verify_header(
    request: &SignedRequest<'_>,
    auth: &AuthorizationHeader,
    secret: &SecretKey,
    server_region: &str,
    now_unix_secs: i64,
) -> AuthResult<Verified> {
    let datetime = AmzDateTime::parse(request.header("x-amz-date").ok_or(AuthError::InvalidDate)?)?;
    auth.scope.check(&datetime, server_region)?;
    let skew = now_unix_secs
        .checked_sub(datetime.unix_secs())
        .map(i64::unsigned_abs);
    if skew.is_none_or(|skew| skew > SIGV4_MAX_CLOCK_SKEW_SECS.unsigned_abs()) {
        return Err(AuthError::ClockSkew);
    }
    let payload_text = request
        .header("x-amz-content-sha256")
        .ok_or(AuthError::MissingContentSha256)?;
    let payload = parse_payload_hash(payload_text)?;
    check_required_signed(request, &auth.signed_headers, true)?;
    let claim = SignatureClaim {
        exclude_query: None,
        signed_headers: &auth.signed_headers,
        payload_hash: payload_text,
        datetime: &datetime,
        scope: &auth.scope,
        signature_hex: &auth.signature,
    };
    let (signing_key, signature) = verify_signature(request, &claim, secret)?;
    Ok(Verified {
        access_key: auth.access_key.clone(),
        scope: auth.scope.clone(),
        datetime,
        signing_key,
        signature,
        payload,
    })
}
