//! Signing in and out. A sign-in is limited per client address BEFORE the credential is looked at, compares the
//! secret of the key it names (the root credential or a user's live key) in constant time, and answers with a session
//! cookie the browser cannot read from script (HttpOnly) and will not send cross-site (SameSite=Strict).

use std::net::SocketAddr;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{ConnectInfo, State};
use axum::http::header::{CONTENT_TYPE, SET_COOKIE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use tessari_s3_constants::{CONSOLE_SESSION_COOKIE, CONSOLE_SESSION_SECS};

use super::ConsoleState;
use super::error::ConsoleError;

#[derive(Deserialize)]
pub(super) struct SignIn {
    access_key_id: String,
    secret_access_key: String,
}

#[derive(Serialize)]
struct Signed<'a> {
    key_id: &'a str,
    expires: i64,
}

/// Whether the request declares a JSON body; a form a cross-site page could post cannot.
pub(super) fn is_json(headers: &HeaderMap) -> bool {
    headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|media| media.trim().eq_ignore_ascii_case("application/json"))
}

fn cookie(state: &ConsoleState, value: &str, max_age: i64) -> Result<HeaderValue, ConsoleError> {
    let secure = if state.secure_cookie() {
        "; Secure"
    } else {
        ""
    };
    HeaderValue::from_str(&format!(
        "{CONSOLE_SESSION_COOKIE}={value}; HttpOnly; SameSite=Strict; Path=/; Max-Age={max_age}{secure}"
    ))
    .map_err(|_| ConsoleError::internal())
}

pub(super) async fn sign_in(
    State(state): State<ConsoleState>,
    ConnectInfo(client): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ConsoleError> {
    let now = state.now();
    if !state
        .sign_ins()
        .admit(&client.ip().to_string(), now)
        .map_err(|()| ConsoleError::internal())?
    {
        return Err(ConsoleError::rate_limited());
    }
    if !is_json(&headers) {
        return Err(ConsoleError::not_json());
    }
    let SignIn {
        access_key_id,
        secret_access_key,
    } = serde_json::from_slice(&body)
        .map_err(|_| ConsoleError::bad_request("expected access_key_id and secret_access_key"))?;
    // The key id names the credential (root or a user's live key); the secret is then compared in constant time.
    let secret_matches = match state.principal_for(&access_key_id).await? {
        Some((_, secret)) => bool::from(
            secret_access_key
                .as_bytes()
                .ct_eq(secret.expose().as_bytes()),
        ),
        None => false,
    };
    if !secret_matches {
        tracing::warn!(client = %client.ip(), "console sign-in refused");
        return Err(ConsoleError::unauthorized());
    }
    let mut nonce = [0_u8; 16];
    getrandom::fill(&mut nonce).map_err(|_| ConsoleError::internal())?;
    let token = tessari_s3_core::console::issue(
        state.secret(),
        &access_key_id,
        now,
        CONSOLE_SESSION_SECS,
        nonce,
    );
    tracing::info!(client = %client.ip(), key_id = access_key_id, "console sign-in");
    let mut response = Json(Signed {
        key_id: &access_key_id,
        expires: now.saturating_add(CONSOLE_SESSION_SECS),
    })
    .into_response();
    response
        .headers_mut()
        .insert(SET_COOKIE, cookie(&state, &token, CONSOLE_SESSION_SECS)?);
    Ok(response)
}

/// Ends the session in this browser. The token itself stays valid until it expires — there is no store to revoke it
/// from — which is why a session is short and changing the root secret ends them all.
pub(super) async fn sign_out(State(state): State<ConsoleState>) -> Result<Response, ConsoleError> {
    let mut response = StatusCode::NO_CONTENT.into_response();
    response
        .headers_mut()
        .insert(SET_COOKIE, cookie(&state, "", 0)?);
    Ok(response)
}
