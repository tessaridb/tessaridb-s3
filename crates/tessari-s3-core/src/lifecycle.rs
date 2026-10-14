//! Bucket lifecycle rules (S3 ref 08 M7): what a rule may say and the checks a configuration passes before it is
//! stored. Transitions and tag or size filters are not offered and are refused where the document is read.

use tessari_s3_types::Timestamp;

/// Most rules one bucket may hold.
pub const MAX_RULES: usize = 1000;
/// Longest rule ID, in characters.
pub const MAX_ID_CHARS: usize = 255;
/// Most noncurrent versions a rule may keep.
pub const MAX_KEEP: u32 = 100;

/// When a current version expires.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expiration {
    /// This many days after it was written.
    Days(u32),
    /// From this instant (midnight UTC).
    Date(Timestamp),
}

/// When noncurrent versions are removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Noncurrent {
    /// Days after a version stopped being current.
    pub days: u32,
    /// How many of the newest noncurrent versions are kept whatever their age.
    pub keep: Option<u32>,
}

/// One rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    /// The rule's ID.
    pub id: String,
    /// `Status` Enabled.
    pub enabled: bool,
    /// Only keys starting with this; empty for every key.
    pub prefix: String,
    /// Expiration of the current version.
    pub expiration: Option<Expiration>,
    /// Remove a delete marker that is its key's only version.
    pub expired_object_delete_marker: bool,
    /// Expiration of noncurrent versions.
    pub noncurrent: Option<Noncurrent>,
    /// Abort multipart uploads this many days after they began.
    pub abort_after_days: Option<u32>,
}

/// Why a configuration is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LifecycleError {
    /// No rule at all.
    #[error("a lifecycle configuration holds at least one rule")]
    NoRules,
    /// More than [`MAX_RULES`].
    #[error("a lifecycle configuration holds at most 1000 rules")]
    TooManyRules,
    /// An ID longer than [`MAX_ID_CHARS`].
    #[error("a rule ID is at most 255 characters")]
    IdTooLong,
    /// Two rules with one ID.
    #[error("rule IDs must be unique")]
    DuplicateId,
    /// A rule with no action.
    #[error("a rule names at least one action")]
    NoAction,
    /// A count of days that is zero.
    #[error("'{0}' must be a positive integer")]
    NotPositive(&'static str),
    /// `NewerNoncurrentVersions` outside 1 to [`MAX_KEEP`].
    #[error("NewerNoncurrentVersions must be from 1 to 100")]
    KeepOutOfRange,
    /// `ExpiredObjectDeleteMarker` beside `Days` or `Date` in one `Expiration`.
    #[error(
        "ExpiredObjectDeleteMarker cannot be specified with Days or Date in a lifecycle expiration policy"
    )]
    MarkerWithDaysOrDate,
}

/// Checks `rules` as S3 does before a configuration is stored.
///
/// # Errors
/// The first [`LifecycleError`] found.
pub fn validate(rules: &[Rule]) -> Result<(), LifecycleError> {
    if rules.is_empty() {
        return Err(LifecycleError::NoRules);
    }
    if rules.len() > MAX_RULES {
        return Err(LifecycleError::TooManyRules);
    }
    let mut ids = std::collections::BTreeSet::new();
    for rule in rules {
        if rule.id.chars().count() > MAX_ID_CHARS {
            return Err(LifecycleError::IdTooLong);
        }
        if !rule.id.is_empty() && !ids.insert(rule.id.as_str()) {
            return Err(LifecycleError::DuplicateId);
        }
        if rule.expiration.is_none()
            && !rule.expired_object_delete_marker
            && rule.noncurrent.is_none()
            && rule.abort_after_days.is_none()
        {
            return Err(LifecycleError::NoAction);
        }
        if rule.expired_object_delete_marker && rule.expiration.is_some() {
            return Err(LifecycleError::MarkerWithDaysOrDate);
        }
        if rule.expiration == Some(Expiration::Days(0)) {
            return Err(LifecycleError::NotPositive("Days"));
        }
        if let Some(noncurrent) = &rule.noncurrent {
            if noncurrent.days == 0 {
                return Err(LifecycleError::NotPositive("NoncurrentDays"));
            }
            if noncurrent
                .keep
                .is_some_and(|keep| keep == 0 || keep > MAX_KEEP)
            {
                return Err(LifecycleError::KeepOutOfRange);
            }
        }
        if rule.abort_after_days == Some(0) {
            return Err(LifecycleError::NotPositive("DaysAfterInitiation"));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;
