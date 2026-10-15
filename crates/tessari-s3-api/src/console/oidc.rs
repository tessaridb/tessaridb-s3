//! Signing in through the identity provider (the relying-party half of OpenID Connect; the console server holds every
//! token, the browser only the session cookie). `start` records a fresh state, nonce and PKCE verifier for
//! [`CONSOLE_OIDC_LOGIN_SECS`] and sends the browser to the provider; `callback` takes that record — once, whatever
//! comes back, so no state is good for a second return — has the provider's answer checked, finds the user the
//! identity is linked to, and issues that user's session. Every refusal is the same answer; the reason goes to the
//! log. Both are limited per client address, like signing in with a key.

use std::net::SocketAddr;

use axum::Json;
use axum::extract::{ConnectInfo, Query, State};
use axum::http::header::{CACHE_CONTROL, LOCATION, SET_COOKIE};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};
use tessari_s3_constants::{
    CONSOLE_FRESH_AUTH_SECS, CONSOLE_OIDC_LOGIN_SECS, CONSOLE_SESSION_SECS,
};
use tessari_s3_core::authz::UserName;
use tessari_s3_core::console::Subject;
use tessari_s3_infrastructure::oidc::{OidcClient, OidcError};
use tessari_s3_storage::identities::PendingLogin;

use super::ConsoleState;
use super::error::ConsoleError;
use super::session::cookie;

/// How a sign-in starts: `fresh=true` asks the provider to make the person authenticate again (`max_age`), as a
/// privileged act requires.
#[derive(Deserialize)]
pub(super) struct Start {
    #[serde(default)]
    fresh: bool,
}

/// What the provider sends the browser back with; anything else in the query is ignored.
#[derive(Deserialize)]
pub(super) struct Return {
    code: Option<String>,
    state: Option<String>,
    iss: Option<String>,
}

/// The routes a sign-in through the provider uses, none behind a session: the start, the callback and the provider's
/// back-channel logout.
pub(super) fn routes() -> axum::Router<ConsoleState> {
    axum::Router::new()
        .route("/api/v1/auth/methods", get(methods))
        .route("/api/v1/auth/oidc/start", get(start))
        .route("/api/v1/auth/oidc/callback", get(callback))
        .route(
            "/api/v1/auth/oidc/backchannel-logout",
            post(super::logout::backchannel),
        )
}

/// How one may sign in here, for the sign-in page: with a key always, and through the provider when one is configured.
#[derive(Serialize)]
pub(super) struct Methods {
    provider: bool,
}

pub(super) async fn methods(State(state): State<ConsoleState>) -> Json<Methods> {
    Json(Methods {
        provider: state.oidc().is_some(),
    })
}

fn admit(state: &ConsoleState, client: &SocketAddr) -> Result<(), ConsoleError> {
    if state
        .sign_ins()
        .admit(&client.ip().to_string(), state.now())
        .map_err(|()| ConsoleError::internal())?
    {
        Ok(())
    } else {
        Err(ConsoleError::rate_limited())
    }
}

/// A `303` to `to` that no cache keeps.
fn redirect(to: &str) -> Result<Response, ConsoleError> {
    let mut response = StatusCode::SEE_OTHER.into_response();
    let headers = response.headers_mut();
    headers.insert(
        LOCATION,
        HeaderValue::from_str(to).map_err(|_| ConsoleError::internal())?,
    );
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

pub(super) async fn start(
    State(state): State<ConsoleState>,
    ConnectInfo(client): ConnectInfo<SocketAddr>,
    Query(start): Query<Start>,
) -> Result<Response, ConsoleError> {
    let oidc = state.oidc().ok_or_else(ConsoleError::not_found)?;
    admit(&state, &client)?;
    let max_age = start
        .fresh
        .then(|| u64::try_from(CONSOLE_FRESH_AUTH_SECS).ok())
        .flatten();
    let started = oidc.begin(max_age).await.map_err(|error| {
        tracing::warn!(error = %error, "console sign-in through the identity provider could not start");
        ConsoleError::unavailable()
    })?;
    state
        .storage()
        .identities()
        .begin_login(
            &PendingLogin {
                state: started.state,
                nonce: started.nonce,
                verifier: started.verifier,
            },
            CONSOLE_OIDC_LOGIN_SECS,
        )
        .await?;
    redirect(&started.url)
}

pub(super) async fn callback(
    State(state): State<ConsoleState>,
    ConnectInfo(client): ConnectInfo<SocketAddr>,
    Query(back): Query<Return>,
) -> Result<Response, ConsoleError> {
    let oidc = state.oidc().ok_or_else(ConsoleError::not_found)?;
    admit(&state, &client)?;
    let (name, authenticated) = match signed_in(&state, oidc, back).await? {
        Ok(signed) => signed,
        Err(reason) => {
            tracing::warn!(client = %client.ip(), reason, "console sign-in through the identity provider refused");
            return Err(ConsoleError::sign_in_refused());
        }
    };
    let mut nonce = [0_u8; 16];
    getrandom::fill(&mut nonce).map_err(|_| ConsoleError::internal())?;
    let subject = Subject::User(name.as_str().to_owned());
    let token = tessari_s3_core::console::issue(
        state.secret(),
        &subject,
        state.now(),
        CONSOLE_SESSION_SECS,
        authenticated,
        nonce,
    );
    tracing::info!(client = %client.ip(), user = name.as_str(), "console sign-in through the identity provider");
    let mut response = redirect("/")?;
    response
        .headers_mut()
        .insert(SET_COOKIE, cookie(&state, &token, CONSOLE_SESSION_SECS)?);
    Ok(response)
}

/// The user a return signs in and when they authenticated at the provider, or the reason it does not; the outer error
/// is the store's or the provider's outage.
async fn signed_in(
    state: &ConsoleState,
    oidc: &OidcClient,
    back: Return,
) -> Result<Result<(UserName, Option<i64>), &'static str>, ConsoleError> {
    let Some(login_state) = back.state else {
        return Ok(Err("no state"));
    };
    // Taken first: from here this state is spent, whether the rest succeeds or not.
    let Some(login) = state
        .storage()
        .identities()
        .take_login(&login_state)
        .await?
    else {
        return Ok(Err("a state never issued, expired or already used"));
    };
    let Some(code) = back.code else {
        return Ok(Err("no code: the provider returned an error"));
    };
    let identity = match oidc
        .finish(&code, back.iss.as_deref(), &login.nonce, &login.verifier)
        .await
    {
        Ok(identity) => identity,
        Err(OidcError::Unavailable) => return Err(ConsoleError::unavailable()),
        Err(error) => {
            tracing::info!(error = %error, "the identity provider's return failed a check");
            return Ok(Err("the provider's return failed a check"));
        }
    };
    let Some(name) = state
        .storage()
        .identities()
        .user_for(&identity.issuer, &identity.subject)
        .await?
    else {
        // The subject is what an operator links; it names no secret.
        tracing::info!(
            subject = identity.subject,
            "an identity linked to no user signed in"
        );
        return Ok(Err("the identity is linked to no user"));
    };
    if state.user_principal(name.as_str()).await?.is_none() {
        return Ok(Err("the linked user is disabled"));
    }
    Ok(Ok((name, identity.authenticated)))
}
