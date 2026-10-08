//! `GET /api/v1/usage`: how many objects and bytes each bucket holds as last measured, the totals, and when the
//! measurement was taken — over the buckets the caller can list. Never measured answers no time and no buckets.

use std::collections::BTreeSet;

use axum::Json;
use axum::extract::{Extension, State};
use serde::Serialize;
use tessari_s3_core::authz::{Principal, Visible};

use super::ConsoleState;
use super::error::ConsoleError;

#[derive(Serialize)]
struct BucketUsageView {
    bucket: String,
    objects: u64,
    bytes: u64,
}

#[derive(Serialize)]
pub(super) struct UsageView {
    taken: Option<String>,
    buckets: Vec<BucketUsageView>,
    objects: u64,
    bytes: u64,
}

pub(super) async fn usage(
    State(state): State<ConsoleState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<UsageView>, ConsoleError> {
    let Some(mut measured) = state.storage().usage().latest().await? else {
        return Ok(Json(UsageView {
            taken: None,
            buckets: Vec::new(),
            objects: 0,
            bytes: 0,
        }));
    };
    // One measurement covers every bucket; a user sees the entries, and totals, of the buckets it can list.
    let visible = principal.visible();
    if visible != Visible::All {
        let names: BTreeSet<String> = state
            .storage()
            .buckets()
            .list(&visible)
            .await?
            .into_iter()
            .map(|bucket| bucket.name.as_str().to_owned())
            .collect();
        measured
            .buckets
            .retain(|bucket| names.contains(&bucket.bucket));
    }
    let objects = measured
        .buckets
        .iter()
        .fold(0_u64, |sum, bucket| sum.saturating_add(bucket.objects));
    let bytes = measured
        .buckets
        .iter()
        .fold(0_u64, |sum, bucket| sum.saturating_add(bucket.bytes));
    Ok(Json(UsageView {
        taken: Some(measured.taken.iso8601_millis()),
        buckets: measured
            .buckets
            .into_iter()
            .map(|bucket| BucketUsageView {
                bucket: bucket.bucket,
                objects: bucket.objects,
                bytes: bucket.bytes,
            })
            .collect(),
        objects,
        bytes,
    }))
}
