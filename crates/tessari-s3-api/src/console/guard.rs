//! The session guard on every protected console route: no session, a tampered one or an expired one is `401`
//! before anything else runs; a valid one is then limited per signed-in key.

use axum::extract::{Request, State};
use axum::http::HeaderMap;
use axum::http::header::COOKIE;
use axum::middleware::Next;
use axum::response::Response;
use tessari_s3_constants::CONSOLE_SESSION_COOKIE;
use tessari_s3_core::console::{Session, verify};

use super::ConsoleState;
use super::error::ConsoleError;

/// The session cookie's value, from any of the request's cookie headers.
fn token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .find_map(|pair| {
            let (name, value) = pair.trim().split_once('=')?;
            (name == CONSOLE_SESSION_COOKIE).then_some(value)
        })
}

pub(super) async fn require_session(
    State(state): State<ConsoleState>,
    mut request: Request,
    next: Next,
) -> Result<Response, ConsoleError> {
    let now = state.now();
    let session: Session = token(request.headers())
        .and_then(|token| verify(state.secret(), token, now).ok())
        .filter(|session| session.key_id == state.key_id())
        .ok_or(ConsoleError::unauthorized())?;
    if !state
        .requests()
        .admit(&session.key_id, now)
        .map_err(|()| ConsoleError::internal())?
    {
        return Err(ConsoleError::rate_limited());
    }
    request.extensions_mut().insert(session);
    Ok(next.run(request).await)
}
