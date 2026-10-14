//! Renditions that failed for good: the tasks in `media_images` and `media_videos` whose hand-outs are spent, with
//! the last error a worker recorded. `GET /api/v1/media/failed`; they span every space, so only operators read them.

use axum::Json;
use axum::extract::{Extension, State};
use serde::Serialize;
use tessari_s3_constants::CONSOLE_MEDIA_FAILED_MAX;
use tessari_s3_core::authz::{self, Principal};
use tessari_s3_storage::media::{Task, TaskQueue};

use super::ConsoleState;
use super::access::allow;
use super::error::ConsoleError;

/// One rendition that is no longer tried.
#[derive(Debug, Serialize)]
pub(super) struct FailedView {
    queue: &'static str,
    bucket: String,
    key: String,
    version_id: Option<String>,
    output: String,
    attempts: i64,
    error: Option<String>,
}

/// The failures, at most [`CONSOLE_MEDIA_FAILED_MAX`] per queue; `more` when a queue holds others.
#[derive(Debug, Serialize)]
pub(super) struct Failed {
    failed: Vec<FailedView>,
    more: bool,
}

fn view(queue: &'static str, task: Task) -> FailedView {
    FailedView {
        queue,
        bucket: task.rendition.bucket,
        key: task.rendition.key,
        version_id: task.rendition.version_id,
        output: task.rendition.output.name(),
        attempts: task.attempts,
        error: task.error,
    }
}

pub(super) async fn list(
    State(state): State<ConsoleState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<Failed>, ConsoleError> {
    allow(&principal, &authz::Action::Operate)?;
    let media = state.storage().media();
    let (mut failed, mut more) = (Vec::new(), false);
    for (queue, name) in [(TaskQueue::Images, "images"), (TaskQueue::Videos, "videos")] {
        // One past the page tells whether the queue holds more.
        let mut tasks = media
            .given_up(queue, CONSOLE_MEDIA_FAILED_MAX.saturating_add(1))
            .await?;
        if tasks.len() > CONSOLE_MEDIA_FAILED_MAX {
            tasks.truncate(CONSOLE_MEDIA_FAILED_MAX);
            more = true;
        }
        failed.extend(tasks.into_iter().map(|task| view(name, task)));
    }
    Ok(Json(Failed { failed, more }))
}
