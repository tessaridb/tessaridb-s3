//! Temporary credentials for a person the console's identity provider vouches for (`AssumeRoleWithWebIdentity`, the
//! STS query protocol on `POST /`): the identity token is believed exactly as far as the console believes an ID token,
//! the role is the user that identity is linked to, and what is issued is a session key of that user — which signs
//! only with its own session token, stops at its expiry and ends with the link, the user or a provider logout.
//! Unauthenticated by design, so it is rate-limited per client address like the console's sign-in.

use axum::body::Body;
use axum::http::request::Parts;
use axum::http::{Response, StatusCode};
use tessari_s3_core::auth::{compose_session_token, session_token_digest};
use tessari_s3_infrastructure::oidc::{OidcClient, OidcError};
use tessari_s3_storage::users::NewSession;
use tessari_s3_types::Timestamp;

use super::address::Addressed;
use super::trail::client;
use crate::console::limits::Window;
use crate::state::ApiState;

mod request;
mod response;

/// The identity provider web identity tokens are checked against, and the per-address budget for asking.
pub(crate) struct WebIdentityIssuer {
    client: OidcClient,
    requests: Window,
}

impl WebIdentityIssuer {
    pub(crate) fn new(client: OidcClient, per_minute: usize) -> Self {
        Self {
            client,
            requests: Window::new(per_minute),
        }
    }
}

/// A refusal in the STS error document's terms.
#[derive(Debug)]
pub(super) struct StsError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl StsError {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }
}

fn rejected(message: &str) -> StsError {
    StsError::new(StatusCode::FORBIDDEN, "IDPRejectedClaim", message)
}

/// A failure of this server's own (the store, the clock, randomness), logged with its cause and answered as
/// `InternalFailure`.
fn failed(error: impl std::fmt::Display) -> StsError {
    tracing::warn!(error = %error, "temporary credentials could not be issued");
    internal()
}

fn internal() -> StsError {
    StsError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "InternalFailure",
        "the request could not be completed",
    )
}

/// What a refused identity token answers; the reason goes to the log, never the token.
fn token_refusal(error: OidcError) -> StsError {
    tracing::info!(reason = %error, "web identity token refused");
    match error {
        OidcError::Unavailable => StsError::new(
            StatusCode::BAD_REQUEST,
            "IDPCommunicationError",
            "the identity provider's keys could not be had",
        ),
        OidcError::IdTokenRefused("expired") => StsError::new(
            StatusCode::BAD_REQUEST,
            "ExpiredTokenException",
            "the web identity token has expired",
        ),
        OidcError::NoSecondFactor => rejected("the sign-in carries no second factor"),
        _ => StsError::new(
            StatusCode::BAD_REQUEST,
            "InvalidIdentityToken",
            "the web identity token could not be verified",
        ),
    }
}

/// Whether this request is an STS call: a form POST to the service root, naming no bucket.
pub(super) fn is_sts(parts: &Parts, addressed: &Addressed) -> bool {
    parts.method == axum::http::Method::POST
        && addressed.bucket.is_none()
        && parts
            .headers
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("application/x-www-form-urlencoded"))
}

/// Answers the STS call in `parts` and `body`, refusals included, as the STS documents.
pub(super) async fn serve(
    state: &ApiState,
    parts: &Parts,
    body: Body,
    request_id: &str,
) -> Response<Body> {
    match assume(state, parts, body, request_id).await {
        Ok(answer) => answer,
        Err(error) => response::refused(&error, request_id),
    }
}

async fn assume(
    state: &ApiState,
    parts: &Parts,
    body: Body,
    request_id: &str,
) -> Result<Response<Body>, StsError> {
    let issuer = state.web_identity().ok_or_else(|| {
        StsError::new(
            StatusCode::NOT_IMPLEMENTED,
            "NotImplemented",
            "no identity provider is configured",
        )
    })?;
    // Requests arriving without a peer address (in-process callers) share one budget.
    let address = client(&parts.extensions).unwrap_or_default();
    if !issuer
        .requests
        .admit(&address, state.now())
        .map_err(|()| internal())?
    {
        return Err(StsError::new(
            StatusCode::BAD_REQUEST,
            "Throttling",
            "too many requests from this address",
        ));
    }
    let bytes = axum::body::to_bytes(body, request::STS_MAX_BODY_BYTES)
        .await
        .map_err(|_| {
            StsError::new(
                StatusCode::BAD_REQUEST,
                "ValidationError",
                "the request body is too large",
            )
        })?;
    let text = std::str::from_utf8(&bytes).map_err(|_| {
        StsError::new(
            StatusCode::BAD_REQUEST,
            "ValidationError",
            "the request body is not UTF-8",
        )
    })?;
    let asked = request::parse(text)?;
    let identity = issuer
        .client
        .verify_web_identity(&asked.token)
        .await
        .map_err(token_refusal)?;
    let storage = state.storage();
    let user = storage
        .identities()
        .user_for(&identity.issuer, &identity.subject)
        .await
        .map_err(failed)?
        .ok_or_else(|| rejected("the identity is not linked to a user"))?;
    let holder = storage
        .users()
        .get(&user)
        .await
        .map_err(failed)?
        .filter(|holder| !holder.disabled)
        .ok_or_else(|| rejected("the user is disabled"))?;
    let role = format!(
        "arn:aws:iam::{}:role/{}",
        holder.space.as_str(),
        user.as_str()
    );
    if asked.role_arn != role {
        return Err(StsError::new(
            StatusCode::FORBIDDEN,
            "AccessDenied",
            "the identity may assume only the role of the user it is linked to",
        ));
    }
    let expires = state
        .now()
        .checked_add(asked.duration_secs)
        .ok_or_else(internal)?;
    let mut random = [0_u8; 32];
    getrandom::fill(&mut random).map_err(failed)?;
    let session_token = compose_session_token(expires, &random);
    let issued = storage
        .users()
        .issue_session(
            &user,
            &NewSession {
                expires,
                token_digest: session_token_digest(&session_token),
                issuer: identity.issuer.clone(),
                subject: identity.subject.clone(),
                policy: asked.policy.clone(),
            },
        )
        .await
        .map_err(failed)?
        .ok_or_else(|| rejected("the user is disabled"))?;
    let expiration = Timestamp {
        seconds: expires,
        nanos: 0,
    }
    .iso8601_millis();
    Ok(response::assumed(
        &response::Assumed {
            access_key_id: &issued.access_key_id,
            secret: issued.secret.expose(),
            session_token: &session_token,
            expiration: &expiration,
            subject: &identity.subject,
            space: holder.space.as_str(),
            user: user.as_str(),
            session_name: &asked.session_name,
            provider: &identity.issuer,
            audience: &identity.audience,
        },
        request_id,
    ))
}
