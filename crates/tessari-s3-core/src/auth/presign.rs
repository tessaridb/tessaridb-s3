//! SigV4 in the query string: presigned URLs.

use tessari_s3_constants::{
    PRESIGN_MAX_EXPIRES_SECS, PRESIGN_MIN_EXPIRES_SECS, SIGV4_MAX_CLOCK_SKEW_SECS,
};

use tessari_s3_types::SecretKey;

use super::canonical::{SignedRequest, check_required_signed};
use super::encode::percent_decode;
use super::signing::{
    ALGORITHM, CredentialScope, SignatureClaim, is_signature_hex, parse_credential,
    parse_signed_headers, verify_signature,
};
use super::time::AmzDateTime;
use super::verified::{PayloadHash, Verified};
use super::{AuthError, AuthResult};

/// The `X-Amz-*` parameters of a presigned URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresignedQuery {
    access_key: String,
    scope: CredentialScope,
    datetime: AmzDateTime,
    expires_secs: i64,
    signed_headers: Vec<String>,
    signature: String,
}

impl PresignedQuery {
    /// Parses the presign parameters out of a raw query; `None` when the query carries no `X-Amz-Algorithm`.
    ///
    /// # Errors
    /// [`AuthError::Malformed`], [`AuthError::UnsupportedAlgorithm`], [`AuthError::InvalidDate`] or
    /// [`AuthError::PresignExpiresOutOfRange`].
    pub fn parse(raw_query: &str) -> AuthResult<Option<Self>> {
        let mut found: [Option<String>; 6] = Default::default();
        const NAMES: [&str; 6] = [
            "X-Amz-Algorithm",
            "X-Amz-Credential",
            "X-Amz-Date",
            "X-Amz-Expires",
            "X-Amz-SignedHeaders",
            "X-Amz-Signature",
        ];
        for parameter in raw_query.split('&').filter(|p| !p.is_empty()) {
            let (raw_name, raw_value) = parameter.split_once('=').unwrap_or((parameter, ""));
            let name = percent_decode(raw_name)?;
            if name == b"X-Amz-Security-Token" {
                // No temporary credentials are issued, so a session token cannot be checked; refuse rather than
                // let a URL outlive a session nobody can see.
                return Err(AuthError::Malformed {
                    reason: "temporary credentials are not supported",
                });
            }
            let Some(index) = NAMES
                .iter()
                .position(|known| known.as_bytes() == name.as_slice())
            else {
                continue;
            };
            let value = String::from_utf8(percent_decode(raw_value)?).map_err(|_| {
                AuthError::Malformed {
                    reason: "presign parameter is not UTF-8",
                }
            })?;
            if found
                .get_mut(index)
                .and_then(|slot| slot.replace(value))
                .is_some()
            {
                return Err(AuthError::Malformed {
                    reason: "presign parameter repeated",
                });
            }
        }
        let [
            algorithm,
            credential,
            date,
            expires,
            signed_headers,
            signature,
        ] = found;
        let Some(algorithm) = algorithm else {
            return Ok(None);
        };
        if algorithm != ALGORITHM {
            return Err(AuthError::UnsupportedAlgorithm);
        }
        let missing = || AuthError::Malformed {
            reason: "presign parameter missing",
        };
        let (access_key, scope) = parse_credential(&credential.ok_or_else(missing)?)?;
        let datetime = AmzDateTime::parse(&date.ok_or_else(missing)?)?;
        let expires_secs = expires
            .ok_or_else(missing)?
            .parse::<i64>()
            .ok()
            .filter(|secs| (PRESIGN_MIN_EXPIRES_SECS..=PRESIGN_MAX_EXPIRES_SECS).contains(secs))
            .ok_or(AuthError::PresignExpiresOutOfRange)?;
        let signed_headers = parse_signed_headers(&signed_headers.ok_or_else(missing)?)?;
        let signature = signature.ok_or_else(missing)?;
        if !is_signature_hex(&signature) {
            return Err(AuthError::Malformed {
                reason: "signature",
            });
        }
        Ok(Some(Self {
            access_key,
            scope,
            datetime,
            expires_secs,
            signed_headers,
            signature,
        }))
    }

    /// The access key: look its secret up, then call [`verify_presigned`].
    #[must_use]
    pub fn access_key(&self) -> &str {
        &self.access_key
    }
}

/// Verifies a presigned request at request time: within its lifetime, not dated in the future beyond the clock-skew
/// window, and signed over the request as it arrived.
///
/// # Errors
/// Any [`AuthError`].
pub fn verify_presigned(
    request: &SignedRequest<'_>,
    query: &PresignedQuery,
    secret: &SecretKey,
    server_region: &str,
    now_unix_secs: i64,
) -> AuthResult<Verified> {
    query.scope.check(&query.datetime, server_region)?;
    let signed_at = query.datetime.unix_secs();
    let valid_from = signed_at.checked_sub(SIGV4_MAX_CLOCK_SKEW_SECS);
    let valid_until = signed_at.checked_add(query.expires_secs);
    let in_lifetime = valid_from
        .zip(valid_until)
        .is_some_and(|(from, until)| (from..=until).contains(&now_unix_secs));
    if !in_lifetime {
        return Err(AuthError::PresignExpired);
    }
    check_required_signed(request, &query.signed_headers, false)?;
    let claim = SignatureClaim {
        exclude_query: Some("X-Amz-Signature"),
        signed_headers: &query.signed_headers,
        payload_hash: "UNSIGNED-PAYLOAD",
        datetime: &query.datetime,
        scope: &query.scope,
        signature_hex: &query.signature,
    };
    let (signing_key, signature) = verify_signature(request, &claim, secret)?;
    Ok(Verified {
        access_key: query.access_key.clone(),
        scope: query.scope.clone(),
        datetime: query.datetime.clone(),
        signing_key,
        signature,
        payload: PayloadHash::Unsigned,
    })
}
