//! The session guard on every protected console route: no session, a tampered one or an expired one is `401`
//! before anything else runs; a valid one is limited per signed-in subject and resolved again — a key through the
//! 5-second principal cache, so a disabled key or user loses its session within that window; a person signed in
//! through the identity provider straight from the store, so a disabled user loses it at the next request, and so
//! does one whose sessions were ended (a logout from the provider, the identity unlinked) after it was issued.

use axum::extract::{Request, State};
use axum::http::HeaderMap;
use axum::http::header::COOKIE;
use axum::middleware::Next;
use axum::response::Response;
use tessari_s3_constants::CONSOLE_SESSION_COOKIE;
use tessari_s3_core::authz::UserName;
use tessari_s3_core::console::{Session, Subject, verify};

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

/// Whether the user `name`'s console sessions were ended at or after `issued`; at the same second the session ends,
/// so a logout never leaves the session it raced.
async fn ended(state: &ConsoleState, name: &str, issued: i64) -> Result<bool, ConsoleError> {
    let Some(user) = UserName::new(name) else {
        return Ok(true);
    };
    Ok(state
        .storage()
        .identities()
        .sessions_ended(&user)
        .await?
        .is_some_and(|at| issued <= at))
}

pub(super) async fn require_session(
    State(state): State<ConsoleState>,
    mut request: Request,
    next: Next,
) -> Result<Response, ConsoleError> {
    let now = state.now();
    let session: Session = token(request.headers())
        .and_then(|token| verify(state.secret(), token, now).ok())
        .ok_or(ConsoleError::unauthorized())?;
    if !state
        .requests()
        .admit(&session.subject.label(), now)
        .map_err(|()| ConsoleError::internal())?
    {
        return Err(ConsoleError::rate_limited());
    }
    let principal = match &session.subject {
        Subject::Key(key_id) => state
            .principal_for(key_id)
            .await?
            .map(|(principal, _)| principal)
            .filter(|principal| state.key_may_sign_in(principal)),
        Subject::User(name) => match state.user_principal(name).await? {
            Some(principal) if !ended(&state, name, session.issued).await? => Some(principal),
            _ => None,
        },
    }
    .ok_or(ConsoleError::unauthorized())?;
    request.extensions_mut().insert(principal);
    request.extensions_mut().insert(session);
    Ok(next.run(request).await)
}
