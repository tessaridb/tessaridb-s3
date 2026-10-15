//! The identity provider's back-channel logout (OpenID Connect Back-Channel Logout 1.0): the provider posts a signed
//! logout token, and when it is believed — every check an ID token would pass, its id never seen before — every
//! console session of the user the identity is linked to ends at its next request. The answer is `200` or `400` and
//! never says why; the reason goes to the log. Limited per client address like signing in.

use std::net::SocketAddr;

use axum::Form;
use axum::extract::{ConnectInfo, State};
use axum::http::header::CACHE_CONTROL;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use tessari_s3_constants::{CONSOLE_OIDC_LOGOUT_AHEAD_SECS, CONSOLE_OIDC_LOGOUT_MAX_AGE_SECS};
use tessari_s3_infrastructure::oidc::OidcError;
use tessari_s3_storage::actions::NewAction;

use super::ConsoleState;
use super::actions::record;
use super::error::ConsoleError;

/// The longest logout token id the console keeps.
const JTI_MAX_CHARS: usize = 255;

#[derive(Deserialize)]
pub(super) struct LogoutForm {
    logout_token: String,
}

fn answer(status: StatusCode) -> Response {
    let mut response = status.into_response();
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

pub(super) async fn backchannel(
    State(state): State<ConsoleState>,
    ConnectInfo(client): ConnectInfo<SocketAddr>,
    Form(form): Form<LogoutForm>,
) -> Result<Response, ConsoleError> {
    let oidc = state.oidc().ok_or_else(ConsoleError::not_found)?;
    if !state
        .sign_ins()
        .admit(&client.ip().to_string(), state.now())
        .map_err(|()| ConsoleError::internal())?
    {
        return Err(ConsoleError::rate_limited());
    }
    let logged_out = match oidc.verify_logout(&form.logout_token).await {
        Ok(logged_out) => logged_out,
        Err(OidcError::Unavailable) => return Err(ConsoleError::unavailable()),
        Err(error) => {
            tracing::warn!(client = %client.ip(), error = %error, "a back-channel logout was refused");
            return Ok(answer(StatusCode::BAD_REQUEST));
        }
    };
    let keep = CONSOLE_OIDC_LOGOUT_MAX_AGE_SECS.saturating_add(CONSOLE_OIDC_LOGOUT_AHEAD_SECS);
    let identities = state.storage().identities();
    if logged_out.jti.chars().count() > JTI_MAX_CHARS
        || !identities.take_logout(&logged_out.jti, keep).await?
    {
        tracing::warn!(client = %client.ip(), "a back-channel logout token arrived again or carried an oversized id");
        return Ok(answer(StatusCode::BAD_REQUEST));
    }
    let Some(user) = identities
        .user_for(&logged_out.issuer, &logged_out.subject)
        .await?
    else {
        tracing::info!(
            subject = logged_out.subject,
            "a back-channel logout named an identity linked to no user"
        );
        return Ok(answer(StatusCode::OK));
    };
    identities.end_sessions(&user, state.now()).await?;
    record(
        &state,
        NewAction {
            operator: "identity provider".to_owned(),
            operation: "provider_logout".to_owned(),
            target: user.as_str().to_owned(),
            reason: None,
            outcome: "done".to_owned(),
        },
    )
    .await?;
    tracing::info!(
        user = user.as_str(),
        "console sessions ended by the identity provider"
    );
    Ok(answer(StatusCode::OK))
}
