//! Authentication: exactly one SigV4 form per request, verified against the credential its access key names. Every
//! refusal is counted in the node's metrics by its reason; a metadata store that cannot answer is not a refusal.

use tessari_s3_core::auth::{
    AuthError, AuthorizationHeader, PayloadHash, PresignedQuery, SignedRequest, Verified,
    parse_payload_hash, verify_header, verify_presigned,
};
use tessari_s3_core::authz::Principal;
use tessari_s3_observability::AuthRefusal;
use tessari_s3_types::ErrorCode;

use crate::state::ApiState;
use crate::{Error, Result};

/// Verifies the request's signature with the secret of the key it names, and answers who signed it. A request with
/// no credentials is the anonymous principal, which reaches only what a bucket policy grants everyone; its body is
/// protected by the digest or unsigned stream it declares, and a signed chunk stream without a signature is refused.
///
/// # Errors
/// `InvalidArgument` for two authentication forms at once,
/// `InvalidAccessKeyId` for an unknown or disabled key, `InvalidToken` / `ExpiredToken` for a session token that does
/// not stand, and the SigV4 refusals.
pub async fn authenticate(
    state: &ApiState,
    request: &SignedRequest<'_>,
) -> Result<(Verified, Principal)> {
    verify(state, request).await.map_err(|(refusal, error)| {
        if let Some(refusal) = refusal {
            state.metrics().auth_refused(refusal);
        }
        error
    })
}

/// Why `error` refused authentication.
const fn reason(error: &AuthError) -> AuthRefusal {
    match error {
        AuthError::ClockSkew => AuthRefusal::Skew,
        AuthError::PresignExpired | AuthError::PresignExpiresOutOfRange => AuthRefusal::Expired,
        AuthError::SignatureMismatch => AuthRefusal::Signature,
        _ => AuthRefusal::Malformed,
    }
}

/// A SigV4 refusal with its reason.
fn refused(error: AuthError) -> (Option<AuthRefusal>, Error) {
    (Some(reason(&error)), error.into())
}

/// A credential lookup's failure: an unknown key or a session token that does not stand is a refusal, the store's
/// outage is not.
fn unresolved(error: Error) -> (Option<AuthRefusal>, Error) {
    let refusal = match error.code {
        ErrorCode::InvalidAccessKeyId => Some(AuthRefusal::UnknownKey),
        ErrorCode::ExpiredToken => Some(AuthRefusal::Expired),
        ErrorCode::InvalidToken => Some(AuthRefusal::Malformed),
        _ => None,
    };
    (refusal, error)
}

async fn verify(
    state: &ApiState,
    request: &SignedRequest<'_>,
) -> std::result::Result<(Verified, Principal), (Option<AuthRefusal>, Error)> {
    let header = request.header("authorization");
    let presigned = PresignedQuery::parse(request.raw_query).map_err(refused)?;
    let now = state.now();
    match (header, presigned) {
        (Some(_), Some(_)) => Err((
            Some(AuthRefusal::Malformed),
            Error::new(
                ErrorCode::InvalidArgument,
                "only one authentication method may be used",
            ),
        )),
        (None, None) => {
            let payload = match request.header("x-amz-content-sha256") {
                None => PayloadHash::Unsigned,
                Some(declared) => parse_payload_hash(declared).map_err(refused)?,
            };
            Ok((
                Verified::anonymous(payload).map_err(refused)?,
                Principal::Anonymous,
            ))
        }
        (Some(value), None) => {
            let parsed = AuthorizationHeader::parse(value).map_err(refused)?;
            let credential = state
                .credential_for(parsed.access_key(), request.header("x-amz-security-token"))
                .await
                .map_err(unresolved)?;
            let verified = verify_header(request, &parsed, &credential.secret, state.region(), now)
                .map_err(refused)?;
            Ok((verified, credential.principal))
        }
        (None, Some(query)) => {
            let credential = state
                .credential_for(query.access_key(), query.security_token())
                .await
                .map_err(unresolved)?;
            let verified =
                verify_presigned(request, &query, &credential.secret, state.region(), now)
                    .map_err(refused)?;
            Ok((verified, credential.principal))
        }
    }
}
