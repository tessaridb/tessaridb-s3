//! Downloading an object through the console. Reading content is the console's one way to reveal data, so it is
//! recorded — before the first byte leaves — with the operator's key id and their reason if they gave one. The bytes
//! are always served as an attachment of an opaque type: a stored `text/html` must never render on the console's
//! own origin, where its script would run with the operator's session.

use axum::body::Body;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Extension, Path, Query, State};
use axum::http::header::{
    CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE, X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use tessari_s3_core::authz::{Action, Principal};
use tessari_s3_core::console::Session;
use tessari_s3_storage::actions::NewAction;
use tessari_s3_storage::objects::Content;
use tessari_s3_types::ErrorCategory;

use super::ConsoleState;
use super::access::on_bucket;
use super::actions::record;
use super::buckets::bucket_name;
use super::error::ConsoleError;
use super::input::reason;
use super::objects::{no_such_bucket, no_such_key, object_key};
use crate::routes::objects::body;

#[derive(Deserialize)]
pub(super) struct DownloadQuery {
    key: Option<String>,
    reason: Option<String>,
}

/// The last segment of `key`, percent-encoded for `filename*` (RFC 5987): only unreserved bytes stay as they are.
fn file_name(key: &str) -> String {
    let last = key
        .rsplit('/')
        .find(|segment| !segment.is_empty())
        .unwrap_or("object");
    let mut encoded = String::with_capacity(last.len());
    for byte in last.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn failed(error: &crate::Error) -> ConsoleError {
    tracing::error!(error = %error, "console download failed before its first byte");
    if error.category() == ErrorCategory::Unavailable {
        ConsoleError::unavailable()
    } else {
        ConsoleError::internal()
    }
}

pub(super) async fn download(
    State(state): State<ConsoleState>,
    Extension(session): Extension<Session>,
    Extension(principal): Extension<Principal>,
    Path(bucket): Path<String>,
    query: Result<Query<DownloadQuery>, QueryRejection>,
) -> Result<Response, ConsoleError> {
    let Query(query) = query.map_err(|_| ConsoleError::bad_request("key is required"))?;
    let why = reason(query.reason, false)?;
    let bucket = bucket_name(&bucket)?;
    on_bucket(&state, &principal, &bucket, Action::ReadObject).await?;
    let key = object_key(query.key.as_deref())?;
    let objects = state.storage().objects();
    let object = match objects.get(&bucket, &key).await? {
        Err(()) => return Err(no_such_bucket()),
        Ok(None) => return Err(no_such_key()),
        Ok(Some(object)) => object,
    };
    let last = object.size.saturating_sub(1);
    // The first block is read and verified here, so damage there is an error answer rather than a short body.
    let content = match &object.content {
        Content::Inline(bytes) => Body::from(bytes.clone()),
        Content::Data(id) => body::send(objects, *id, object.size, 0, last)
            .await
            .map_err(|error| failed(&error))?,
        Content::Parts(multipart) => {
            let parts: Vec<([u8; 16], u64)> = multipart
                .parts
                .iter()
                .map(|part| (part.data, part.size))
                .collect();
            body::send_parts(objects, &parts, 0, last)
                .await
                .map_err(|error| failed(&error))?
        }
    };
    record(
        &state,
        NewAction {
            operator: session.key_id,
            operation: "download_object".to_owned(),
            target: format!("{}/{}", bucket.as_str(), key.as_str()),
            reason: why,
            outcome: "sent".to_owned(),
        },
    )
    .await?;
    let disposition = HeaderValue::from_str(&format!(
        "attachment; filename*=UTF-8''{}",
        file_name(key.as_str())
    ))
    .map_err(|_| ConsoleError::internal())?;
    let mut response = (StatusCode::OK, content).into_response();
    let headers = response.headers_mut();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    headers.insert(CONTENT_LENGTH, HeaderValue::from(object.size));
    headers.insert(CONTENT_DISPOSITION, disposition);
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    Ok(response)
}
