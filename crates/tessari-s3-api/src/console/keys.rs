//! Issuing a user an access key through the console. The secret is in this one answer, marked never to be stored by
//! the browser or a cache, and nowhere else: not in the action record, not in a log, not in any later listing.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use tessari_s3_core::authz::Principal;
use tessari_s3_core::console::Session;
use tessari_s3_storage::actions::NewAction;

use super::ConsoleState;
use super::actions::record;
use super::error::ConsoleError;
use super::input::{json, reason};
use super::users::managed;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Issue {
    reason: Option<String>,
}

#[derive(Serialize)]
struct IssuedView<'a> {
    access_key_id: &'a str,
    secret_access_key: &'a str,
}

pub(super) async fn issue(
    State(state): State<ConsoleState>,
    Extension(session): Extension<Session>,
    Extension(principal): Extension<Principal>,
    Path(name): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ConsoleError> {
    let Issue { reason: given } = json(&headers, &body, "expected reason")?;
    let why = reason(given, true)?;
    let user = managed(&state, &principal, &name).await?;
    let issued = match state.storage().users().issue_key(&user.name).await {
        Err(tessari_s3_storage::Error::NoIamKey) => {
            return Err(ConsoleError::conflict(
                "no_iam_key",
                "this server has no TESSARIDB_S3_IAM_KEY, so it cannot issue keys",
            ));
        }
        other => other?,
    };
    record(
        &state,
        NewAction {
            operator: session.key_id,
            operation: "issue_key".to_owned(),
            target: user.name.as_str().to_owned(),
            reason: why,
            outcome: issued
                .as_ref()
                .map_or("no_such_user", |key| key.access_key_id.as_str())
                .to_owned(),
        },
    )
    .await?;
    let Some(key) = issued else {
        return Err(ConsoleError::missing(
            "no_such_user",
            "there is no such user",
        ));
    };
    let mut response = (
        StatusCode::CREATED,
        Json(IssuedView {
            access_key_id: &key.access_key_id,
            secret_access_key: key.secret.expose(),
        }),
    )
        .into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}
