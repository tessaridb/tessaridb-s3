//! Issuing a one-key upload credential through the console (ADR-0007 rule 8): a user who may write a bucket hands out a
//! key that uploads one named key of it until it expires, and only while that user may still write there. The issuer
//! is a user because the key's authority is the issuer's, narrowed: root's authority cannot be taken away, so a key
//! delegated from it would be a bearer credential with nothing to narrow. The secret is in this one answer, marked
//! never to be stored, and nowhere else.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use tessari_s3_constants::{UPLOAD_KEY_MAX_SECS, UPLOAD_KEY_MIN_SECS};
use tessari_s3_core::authz::{Action, Principal, UserName};
use tessari_s3_core::console::Session;
use tessari_s3_storage::actions::NewAction;
use tessari_s3_storage::users::KeyScope;
use tessari_s3_types::{ObjectKey, Timestamp};

use super::ConsoleState;
use super::access::on_bucket;
use super::actions::record;
use super::buckets::bucket_name;
use super::error::ConsoleError;
use super::input::{json, reason};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Issue {
    key: String,
    expires_in: i64,
    reason: Option<String>,
}

#[derive(Serialize)]
struct IssuedView<'a> {
    access_key_id: &'a str,
    secret_access_key: &'a str,
    expires: String,
}

pub(super) async fn issue(
    State(state): State<ConsoleState>,
    Extension(session): Extension<Session>,
    Extension(principal): Extension<Principal>,
    Path(bucket): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ConsoleError> {
    let Issue {
        key,
        expires_in,
        reason: given,
    } = json(&headers, &body, "expected key, expires_in and reason")?;
    let why = reason(given, true)?;
    let name = bucket_name(&bucket)?;
    let key = ObjectKey::new(&key)
        .map_err(|_| ConsoleError::invalid("invalid_key", "a key is 1-1024 bytes of text"))?;
    let expires = state
        .now()
        .checked_add(expires_in)
        .filter(|_| (UPLOAD_KEY_MIN_SECS..=UPLOAD_KEY_MAX_SECS).contains(&expires_in))
        .ok_or(ConsoleError::invalid(
            "invalid_expiry",
            "expires_in is 60 to 604800 seconds",
        ))?;
    let Principal::User(issuer) = &principal else {
        return Err(ConsoleError::refused(
            "issued_by_a_user",
            "an upload key is issued by a user, whose authority it narrows; root has none to narrow",
        ));
    };
    on_bucket(&state, &principal, &name, Action::WriteObject).await?;
    let user = UserName::new(&issuer.name).ok_or(ConsoleError::forbidden())?;
    let scope = KeyScope {
        bucket: name,
        key,
        expires,
    };
    let issued = match state
        .storage()
        .users()
        .issue_upload_key(&user, &scope)
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
    // The issuer resolved a moment ago; gone now, it may issue nothing.
    let key = issued.ok_or(ConsoleError::forbidden())?;
    record(
        &state,
        NewAction {
            operator: session.key_id,
            operation: "issue_upload_key".to_owned(),
            target: format!("{}/{}", scope.bucket.as_str(), scope.key.as_str()),
            reason: why,
            outcome: key.access_key_id.clone(),
        },
    )
    .await?;
    let expires = Timestamp {
        seconds: scope.expires,
        nanos: 0,
    }
    .iso8601_millis();
    let mut response = (
        StatusCode::CREATED,
        Json(IssuedView {
            access_key_id: &key.access_key_id,
            secret_access_key: key.secret.expose(),
            expires,
        }),
    )
        .into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}
