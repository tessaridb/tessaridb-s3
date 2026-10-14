//! A bucket's media rules through the console (G004 C1): read by whoever reads the bucket, set by whoever configures
//! it, checked as a whole and stored whole; every change is recorded.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use tessari_s3_core::authz::{Action, Principal};
use tessari_s3_core::console::Session;
use tessari_s3_core::media::{MediaError, Output, Rule, validate};
use tessari_s3_storage::actions::NewAction;

use super::ConsoleState;
use super::access::on_bucket;
use super::actions::record;
use super::buckets::bucket_name;
use super::error::ConsoleError;
use super::input::{json, reason};

/// One rule as the console sends and shows it.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MediaRule {
    id: String,
    content_types: Vec<String>,
    prefix: String,
    outputs: Vec<String>,
}

#[derive(Serialize)]
pub(super) struct MediaRules {
    rules: Vec<MediaRule>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SetMedia {
    rules: Vec<MediaRule>,
    reason: Option<String>,
}

/// A refusal in the console's own words: the message names what is wrong and never repeats the caller's input.
fn refused(error: &MediaError) -> ConsoleError {
    let message = match error {
        MediaError::NoContentType => "a media rule names at least one content type",
        MediaError::NoOutput => "a media rule names at least one output",
        MediaError::BadContentType(_) => "a content type is type/subtype or type/*",
        MediaError::WrongKind(_) => {
            "an output cannot be made from every content type its rule takes"
        }
        MediaError::DuplicateId => "media rule IDs must be unique",
        MediaError::RepeatedOutput => "a media rule names each output once",
        MediaError::RenditionPrefix => {
            "a media rule cannot take keys under a '.renditions/' segment"
        }
        MediaError::TooManyRules => "a bucket holds at most 100 media rules",
    };
    ConsoleError::invalid("invalid_media", message)
}

pub(super) async fn get(
    State(state): State<ConsoleState>,
    Extension(principal): Extension<Principal>,
    Path(bucket): Path<String>,
) -> Result<Json<MediaRules>, ConsoleError> {
    let name = bucket_name(&bucket)?;
    on_bucket(&state, &principal, &name, Action::ReadBucket).await?;
    let found = state
        .storage()
        .buckets()
        .get(&name)
        .await?
        .ok_or_else(|| ConsoleError::missing("no_such_bucket", "the bucket does not exist"))?;
    let rules = found
        .media
        .unwrap_or_default()
        .into_iter()
        .map(|rule| MediaRule {
            outputs: rule.outputs.iter().map(|output| output.name()).collect(),
            id: rule.id,
            content_types: rule.content_types,
            prefix: rule.prefix,
        })
        .collect();
    Ok(Json(MediaRules { rules }))
}

pub(super) async fn set(
    State(state): State<ConsoleState>,
    Extension(session): Extension<Session>,
    Extension(principal): Extension<Principal>,
    Path(bucket): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ConsoleError> {
    let SetMedia {
        rules,
        reason: given,
    } = json(&headers, &body, "expected rules and an optional reason")?;
    let why = reason(given, false)?;
    let name = bucket_name(&bucket)?;
    on_bucket(&state, &principal, &name, Action::ConfigureBucket).await?;
    let rules = rules
        .into_iter()
        .map(|rule| {
            let outputs = rule
                .outputs
                .iter()
                .map(|output| {
                    Output::parse(output).ok_or(ConsoleError::invalid(
                        "invalid_media",
                        "an output is image, video or poster, a listed size and a listed format",
                    ))
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Rule {
                id: rule.id,
                content_types: rule.content_types,
                prefix: rule.prefix,
                outputs,
            })
        })
        .collect::<Result<Vec<_>, ConsoleError>>()?;
    validate(&rules).map_err(|error| refused(&error))?;
    let set = state.storage().buckets().set_media(&name, &rules).await?;
    record(
        &state,
        NewAction {
            operator: session.subject.operator(),
            operation: "set_media".to_owned(),
            target: name.as_str().to_owned(),
            reason: why,
            outcome: if set { "done" } else { "no_such_bucket" }.to_owned(),
        },
    )
    .await?;
    if set {
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Err(ConsoleError::missing(
            "no_such_bucket",
            "the bucket does not exist",
        ))
    }
}
