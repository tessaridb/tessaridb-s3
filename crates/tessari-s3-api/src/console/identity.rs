//! Linking a user to the identity a person signs in with at the identity provider: whoever may manage the user may
//! link it — never themselves — and every change is recorded with its reason. An identity another user holds is not
//! taken from them, and a user holds one identity at a time.

use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use tessari_s3_core::authz::Principal;
use tessari_s3_core::console::Session;
use tessari_s3_storage::actions::NewAction;
use tessari_s3_storage::identities::Linked;

use super::ConsoleState;
use super::actions::record;
use super::error::ConsoleError;
use super::input::{json, reason};
use super::users::managed;

/// The longest subject OpenID Connect allows.
const SUBJECT_MAX_CHARS: usize = 255;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LinkRequest {
    subject: String,
    reason: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct UnlinkRequest {
    reason: Option<String>,
}

pub(super) async fn link(
    State(state): State<ConsoleState>,
    Extension(session): Extension<Session>,
    Extension(principal): Extension<Principal>,
    Path(name): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ConsoleError> {
    super::access::recently_authenticated(&state, &session)?;
    let oidc = state.oidc().ok_or_else(ConsoleError::not_found)?;
    let LinkRequest {
        subject,
        reason: given,
    } = json(&headers, &body, "expected subject and reason")?;
    let fits = (1..=SUBJECT_MAX_CHARS).contains(&subject.chars().count())
        && !subject.chars().any(char::is_control);
    if !fits {
        return Err(ConsoleError::invalid(
            "invalid_subject",
            "a subject is 1-255 characters with no control characters",
        ));
    }
    let why = reason(given, true)?;
    let user = managed(&state, &principal, &name).await?;
    let linked = state
        .storage()
        .identities()
        .link(&user.name, oidc.issuer(), &subject)
        .await?;
    record(
        &state,
        NewAction {
            operator: session.subject.operator(),
            operation: "link_identity".to_owned(),
            target: user.name.as_str().to_owned(),
            reason: why,
            outcome: match linked {
                Linked::Linked => "done",
                Linked::TakenByAnother => "identity_taken",
            }
            .to_owned(),
        },
    )
    .await?;
    match linked {
        Linked::Linked => Ok(StatusCode::NO_CONTENT.into_response()),
        Linked::TakenByAnother => Err(ConsoleError::conflict(
            "identity_taken",
            "another user holds that identity",
        )),
    }
}

pub(super) async fn unlink(
    State(state): State<ConsoleState>,
    Extension(session): Extension<Session>,
    Extension(principal): Extension<Principal>,
    Path(name): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ConsoleError> {
    state.oidc().ok_or_else(ConsoleError::not_found)?;
    let UnlinkRequest { reason: given } = json(&headers, &body, "expected reason")?;
    let why = reason(given, true)?;
    let user = managed(&state, &principal, &name).await?;
    let removed = state.storage().identities().unlink(&user.name).await?;
    if removed {
        // The link is how the person got in: the sessions it opened end with it.
        state
            .storage()
            .identities()
            .end_sessions(&user.name, state.now())
            .await?;
    }
    record(
        &state,
        NewAction {
            operator: session.subject.operator(),
            operation: "unlink_identity".to_owned(),
            target: user.name.as_str().to_owned(),
            reason: why,
            outcome: if removed { "done" } else { "no_identity" }.to_owned(),
        },
    )
    .await?;
    if removed {
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Err(ConsoleError::missing(
            "no_identity",
            "the user has no linked identity",
        ))
    }
}
