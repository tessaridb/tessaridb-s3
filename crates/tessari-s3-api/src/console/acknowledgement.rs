//! Who acknowledges a bucket's object writes, through the console: operators only, with a reason that is recorded.
//! `region` asks for the bucket's region's majority alone — faster when the region's metadata is also kept in other
//! regions, and a write acknowledged that way can be lost to a failover of that metadata's leadership; `majority` is
//! the default. The store is asked to prove the region level before a bucket is set to it, and its refusal is shown
//! in its own words.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use tessari_s3_core::authz::policy::Context;
use tessari_s3_core::authz::{Action, Principal};
use tessari_s3_core::console::Session;
use tessari_s3_storage::actions::NewAction;
use tessari_s3_storage::buckets::{Acknowledgement, AcknowledgementSet};

use super::ConsoleState;
use super::access::{allow, on_bucket};
use super::actions::record;
use super::buckets::bucket_name;
use super::error::ConsoleError;
use super::input::{json, reason};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SetAcknowledgement {
    level: Option<String>,
    reason: Option<String>,
}

/// The store's own refusal of the region level, which the operator needs verbatim to act on.
#[derive(Serialize)]
struct Refused {
    code: &'static str,
    message: String,
}

pub(super) async fn set(
    State(state): State<ConsoleState>,
    Extension(session): Extension<Session>,
    Extension(principal): Extension<Principal>,
    Extension(context): Extension<Context>,
    Path(bucket): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ConsoleError> {
    super::access::recently_authenticated(&state, &session)?;
    let SetAcknowledgement {
        level,
        reason: given,
    } = json(&headers, &body, "expected level and reason")?;
    let why = reason(given, true)?;
    allow(&principal, &Action::Operate)?;
    let acknowledgement = match level.as_deref() {
        Some("majority") => Acknowledgement::Majority,
        Some("region") => Acknowledgement::RegionMajority,
        _ => {
            return Err(ConsoleError::invalid(
                "invalid_level",
                "level is majority or region",
            ));
        }
    };
    let name = bucket_name(&bucket)?;
    on_bucket(
        &state,
        &principal,
        &context,
        &name,
        Action::ConfigureBucket,
        ("s3:PutBucketAcknowledgement", None),
    )
    .await?;
    let set = state
        .storage()
        .buckets()
        .set_acknowledgement(&name, acknowledgement)
        .await?;
    let outcome = match &set {
        AcknowledgementSet::Set => "done",
        AcknowledgementSet::NoSuchBucket => "no_such_bucket",
        AcknowledgementSet::Refused(_) => "refused",
    };
    record(
        &state,
        NewAction {
            operator: session.subject.operator(),
            operation: format!("set_acknowledgement_{}", level.unwrap_or_default()),
            target: name.as_str().to_owned(),
            reason: why,
            outcome: outcome.to_owned(),
        },
    )
    .await?;
    match set {
        AcknowledgementSet::Set => Ok(StatusCode::NO_CONTENT.into_response()),
        AcknowledgementSet::NoSuchBucket => Err(ConsoleError::missing(
            "no_such_bucket",
            "there is no such bucket",
        )),
        AcknowledgementSet::Refused(message) => Ok((
            StatusCode::CONFLICT,
            Json(Refused {
                code: "acknowledgement_refused",
                message,
            }),
        )
            .into_response()),
    }
}
