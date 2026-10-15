//! What a console request carries in: a JSON body (a form a cross-site page could post is refused before it is read)
//! and an operator's reason, trimmed and bounded.

use axum::http::HeaderMap;
use serde::de::DeserializeOwned;
use tessari_s3_constants::CONSOLE_REASON_MAX_CHARS;

use super::error::ConsoleError;
use super::session::is_json;

/// The body as `T`; `expected` names its fields for the refusal.
pub(super) fn json<T: DeserializeOwned>(
    headers: &HeaderMap,
    body: &[u8],
    expected: &'static str,
) -> Result<T, ConsoleError> {
    if !is_json(headers) {
        return Err(ConsoleError::not_json());
    }
    serde_json::from_slice(body).map_err(|_| ConsoleError::bad_request(expected))
}

/// The operator's reason, trimmed: refused when `required` and empty, or when longer than the limit.
pub(super) fn reason(text: Option<String>, required: bool) -> Result<Option<String>, ConsoleError> {
    let reason = text
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty());
    match reason {
        None if required => Err(ConsoleError::invalid(
            "reason_required",
            "say why: a reason is required for this action",
        )),
        Some(text) if text.chars().count() > CONSOLE_REASON_MAX_CHARS => Err(
            ConsoleError::invalid("reason_too_long", "a reason is at most 500 characters"),
        ),
        reason => Ok(reason),
    }
}
