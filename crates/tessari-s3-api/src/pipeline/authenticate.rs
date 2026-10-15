//! Authentication: exactly one SigV4 form per request, verified against the credential its access key names.

use tessari_s3_core::auth::{
    AuthorizationHeader, PresignedQuery, SignedRequest, Verified, verify_header, verify_presigned,
};
use tessari_s3_core::authz::Principal;
use tessari_s3_types::ErrorCode;

use crate::state::ApiState;
use crate::{Error, Result};

/// Verifies the request's signature with the secret of the key it names, and answers who signed it. Anonymous
/// requests are refused: no bucket policy can grant public access yet.
///
/// # Errors
/// `AccessDenied` for an anonymous request, `InvalidArgument` for two authentication forms at once,
/// `InvalidAccessKeyId` for an unknown or disabled key, and the SigV4 refusals.
pub async fn authenticate(
    state: &ApiState,
    request: &SignedRequest<'_>,
) -> Result<(Verified, Principal)> {
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
            let credential = state.credential_for(parsed.access_key()).await?;
            let verified =
                verify_header(request, &parsed, &credential.secret, state.region(), now)?;
            Ok((verified, credential.principal))
        }
        (None, Some(query)) => {
            let credential = state.credential_for(query.access_key()).await?;
            let verified =
                verify_presigned(request, &query, &credential.secret, state.region(), now)?;
            Ok((verified, credential.principal))
        }
    }
}
