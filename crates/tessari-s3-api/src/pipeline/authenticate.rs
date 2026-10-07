//! Authentication: exactly one SigV4 form per request, verified against the credential its access key names.

use tessari_s3_core::auth::{
    AuthorizationHeader, PresignedQuery, SignedRequest, Verified, verify_header, verify_presigned,
};
use tessari_s3_types::ErrorCode;

use crate::state::ApiState;
use crate::{Error, Result};

/// Verifies the request's signature. Anonymous requests are refused: no bucket policy can grant public access yet.
///
/// # Errors
/// `AccessDenied` for an anonymous request, `InvalidArgument` for two authentication forms at once,
/// `InvalidAccessKeyId` for an unknown key, and the SigV4 refusals.
pub fn authenticate(state: &ApiState, request: &SignedRequest<'_>) -> Result<Verified> {
    let header = request.header("authorization");
    let presigned = PresignedQuery::parse(request.raw_query)?;
    let now = state.now();
    match (header, presigned) {
        (Some(_), Some(_)) => Err(Error::new(
            ErrorCode::InvalidArgument,
            "only one authentication method may be used",
        )),
        (None, None) => Err(Error::new(
            ErrorCode::AccessDenied,
            "anonymous access is not allowed",
        )),
        (Some(value), None) => {
            if request.header("x-amz-security-token").is_some() {
                return Err(Error::new(
                    ErrorCode::InvalidArgument,
                    "temporary credentials are not supported",
                ));
            }
            let parsed = AuthorizationHeader::parse(value)?;
            let secret = state.secret_for(parsed.access_key())?;
            Ok(verify_header(
                request,
                &parsed,
                secret,
                state.region(),
                now,
            )?)
        }
        (None, Some(query)) => {
            let secret = state.secret_for(query.access_key())?;
            Ok(verify_presigned(
                request,
                &query,
                secret,
                state.region(),
                now,
            )?)
        }
    }
}
