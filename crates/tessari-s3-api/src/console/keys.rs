//! A user's access keys through the console: issued, listed and revoked. The secret is in the one answer that issued
//! it, marked never to be stored by the browser or a cache, and nowhere else: not in the action record, not in a log,
//! not in a listing, and not in the answer to a repeated submission. Issuing takes a token the form minted when it
//! opened, so a submission sent twice (a lost answer, a double click) issues one key.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use tessari_s3_core::authz::Principal;
use tessari_s3_core::console::Session;
use tessari_s3_storage::actions::NewAction;
use tessari_s3_storage::users::{IssueToken, IssuedOnce, ListedKey};

use super::ConsoleState;
use super::actions::record;
use super::error::ConsoleError;
use super::input::{json, reason};
use super::users::managed;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Issue {
    reason: Option<String>,
    idempotency_key: Option<String>,
}

#[derive(Serialize)]
struct IssuedView<'a> {
    access_key_id: &'a str,
    secret_access_key: &'a str,
}

#[derive(Serialize)]
struct AlreadyIssuedView<'a> {
    access_key_id: &'a str,
    already_issued: bool,
}

#[derive(Serialize)]
struct KeysView {
    keys: Vec<KeyView>,
}

#[derive(Serialize)]
struct KeyView {
    access_key_id: String,
    created: String,
    disabled: bool,
    upload: Option<UploadView>,
}

/// A one-key credential's reach.
#[derive(Serialize)]
struct UploadView {
    bucket: String,
    key: String,
    expires: i64,
}

impl From<ListedKey> for KeyView {
    fn from(key: ListedKey) -> Self {
        Self {
            access_key_id: key.access_key_id,
            created: key.created.iso8601_millis(),
            disabled: key.disabled,
            upload: key.scope.map(|scope| UploadView {
                bucket: scope.bucket.as_str().to_owned(),
                key: scope.key.as_str().to_owned(),
                expires: scope.expires,
            }),
        }
    }
}

/// An answer no browser or cache keeps.
fn unstored(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

pub(super) async fn issue(
    State(state): State<ConsoleState>,
    Extension(session): Extension<Session>,
    Extension(principal): Extension<Principal>,
    Path(name): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ConsoleError> {
    let Issue {
        reason: given,
        idempotency_key,
    } = json(&headers, &body, "expected reason and idempotency_key")?;
    let why = reason(given, true)?;
    let token = match idempotency_key {
        None => {
            return Err(ConsoleError::invalid(
                "idempotency_key_required",
                "an issue carries the idempotency_key its form minted when it opened",
            ));
        }
        Some(text) => IssueToken::new(&text).ok_or(ConsoleError::invalid(
            "invalid_idempotency_key",
            "an idempotency_key is 16-64 letters, digits and hyphens",
        ))?,
    };
    let user = managed(&state, &principal, &name).await?;
    let issued = match state
        .storage()
        .users()
        .issue_key_once(&user.name, &token)
        .await
    {
        Err(tessari_s3_storage::Error::NoIamKey) => {
            return Err(ConsoleError::conflict(
                "no_iam_key",
                "this server has no TESSARIDB_S3_IAM_KEY, so it cannot issue keys",
            ));
        }
        other => other?,
    };
    let outcome = match &issued {
        IssuedOnce::Issued(key) => key.access_key_id.clone(),
        IssuedOnce::AlreadyIssued(_) => "already_issued".to_owned(),
        IssuedOnce::TokenReused => "idempotency_key_reused".to_owned(),
        IssuedOnce::NoSuchUser => "no_such_user".to_owned(),
    };
    record(
        &state,
        NewAction {
            operator: session.key_id,
            operation: "issue_key".to_owned(),
            target: user.name.as_str().to_owned(),
            reason: why,
            outcome,
        },
    )
    .await?;
    match issued {
        IssuedOnce::Issued(key) => Ok(unstored(
            (
                StatusCode::CREATED,
                Json(IssuedView {
                    access_key_id: &key.access_key_id,
                    secret_access_key: key.secret.expose(),
                }),
            )
                .into_response(),
        )),
        IssuedOnce::AlreadyIssued(id) => Ok(unstored(
            Json(AlreadyIssuedView {
                access_key_id: &id,
                already_issued: true,
            })
            .into_response(),
        )),
        IssuedOnce::TokenReused => Err(ConsoleError::conflict(
            "idempotency_key_reused",
            "this idempotency_key already issued a key to another user",
        )),
        IssuedOnce::NoSuchUser => Err(ConsoleError::missing(
            "no_such_user",
            "there is no such user",
        )),
    }
}

/// The user's keys, without their secrets.
pub(super) async fn list(
    State(state): State<ConsoleState>,
    Extension(principal): Extension<Principal>,
    Path(name): Path<String>,
) -> Result<Response, ConsoleError> {
    let user = managed(&state, &principal, &name).await?;
    let keys = state.storage().users().keys_of(&user.name).await?;
    Ok(Json(KeysView {
        keys: keys.into_iter().map(KeyView::from).collect(),
    })
    .into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Revoke {
    reason: Option<String>,
}

/// Revokes one of the user's keys. Every node believes a key's cached resolution for the published window
/// (`principals::WINDOW_SECS`), so the key is refused everywhere once that much time has passed.
pub(super) async fn revoke(
    State(state): State<ConsoleState>,
    Extension(session): Extension<Session>,
    Extension(principal): Extension<Principal>,
    Path((name, key)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ConsoleError> {
    let Revoke { reason: given } = json(&headers, &body, "expected reason")?;
    let why = reason(given, true)?;
    let user = managed(&state, &principal, &name).await?;
    let revoked = state
        .storage()
        .users()
        .revoke_key_of(&user.name, &key)
        .await?;
    record(
        &state,
        NewAction {
            operator: session.key_id,
            operation: "revoke_key".to_owned(),
            target: user.name.as_str().to_owned(),
            reason: why,
            outcome: if revoked {
                key
            } else {
                "no_such_key".to_owned()
            },
        },
    )
    .await?;
    if revoked {
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Err(ConsoleError::missing(
            "no_such_key",
            "the user has no such key",
        ))
    }
}
