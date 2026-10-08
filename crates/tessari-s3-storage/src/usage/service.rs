//! Usage rules: one member measures at a time, by a claim the store expires, and the console reads what was stored.

use super::model::Usage;
use super::repository::UsageRepository;
use super::tessaridb::TessariUsage;
use crate::Result;

/// Usage operations.
#[derive(Clone)]
pub struct UsageService {
    repository: TessariUsage,
}

impl UsageService {
    /// The service over `repository`.
    #[must_use]
    pub const fn new(repository: TessariUsage) -> Self {
        Self { repository }
    }

    /// Measures every bucket and stores the result, unless another member holds the claim. Answers how many buckets
    /// were measured, or `None` when this pass was another member's.
    ///
    /// # Errors
    /// The metadata store's refusal or outage, or a malformed answer.
    pub async fn refresh(&self, holder: &str) -> Result<Option<usize>> {
        refresh(&self.repository, holder).await
    }

    /// The latest measurement, if any was taken.
    ///
    /// # Errors
    /// The metadata store's refusal or outage, or a malformed record.
    pub async fn latest(&self) -> Result<Option<Usage>> {
        self.repository
            .latest()
            .await?
            .map(Usage::try_from)
            .transpose()
    }
}

pub(super) async fn refresh(
    repository: &impl UsageRepository,
    holder: &str,
) -> Result<Option<usize>> {
    if !repository.claim(holder).await? {
        return Ok(None);
    }
    let buckets = repository.measure().await?;
    repository.store(&buckets).await?;
    Ok(Some(buckets.len()))
}
