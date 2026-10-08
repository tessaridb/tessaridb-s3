//! Action-record rules: an action is recorded as it ended, and the record is read newest first a page at a time.

use tessari_s3_constants::CONSOLE_ACTIONS_PAGE_MAX;

use super::model::{Action, NewAction};
use super::repository::ActionRepository;
use super::tessaridb::TessariActions;
use crate::Result;

/// Action-record operations.
#[derive(Clone)]
pub struct ActionService {
    repository: TessariActions,
}

impl ActionService {
    /// The service over `repository`.
    #[must_use]
    pub const fn new(repository: TessariActions) -> Self {
        Self { repository }
    }

    /// Records `action`.
    ///
    /// # Errors
    /// The metadata store's refusal or outage.
    pub async fn record(&self, action: &NewAction) -> Result<()> {
        self.repository.append(action).await
    }

    /// Up to `limit` actions (at most [`CONSOLE_ACTIONS_PAGE_MAX`]) before position `before` — or the newest when it
    /// is `None` — newest first. The last one's position is the next page's `before`.
    ///
    /// # Errors
    /// The metadata store's refusal or outage, or a malformed message.
    pub async fn recent(&self, before: Option<u64>, limit: usize) -> Result<Vec<Action>> {
        let limit = limit.clamp(1, CONSOLE_ACTIONS_PAGE_MAX);
        let last = self.repository.last().await?;
        let end = before.map_or(last, |before| before.saturating_sub(1).min(last));
        let count = u64::try_from(limit).unwrap_or(u64::MAX).min(end);
        let mut actions = self
            .repository
            .after(end.saturating_sub(count), limit)
            .await?
            .into_iter()
            .map(Action::try_from)
            .collect::<Result<Vec<_>>>()?;
        // Messages appended after `last` was read belong to a newer page.
        actions.retain(|action| action.position <= end);
        actions.reverse();
        Ok(actions)
    }
}
