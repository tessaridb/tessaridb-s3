//! Bucket replication rules (S3 ref 11 M4): what a rule may say and the checks a configuration passes before it is
//! stored. Only the `Filter` form of a rule is offered — a prefix, a priority, a destination bucket and whether delete
//! markers follow; tag filters, storage classes, encryption and time control are refused where the document is read.

/// Most rules one bucket may hold.
pub const MAX_RULES: usize = 1000;
/// Longest rule ID, in characters.
pub const MAX_ID_CHARS: usize = 255;

/// One rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    /// The rule's ID.
    pub id: String,
    /// `Status` Enabled.
    pub enabled: bool,
    /// Where several rules match one key, the highest priority sends it.
    pub priority: u32,
    /// Only keys starting with this; empty for every key.
    pub prefix: String,
    /// The bucket versions are replicated into.
    pub destination: String,
    /// `DeleteMarkerReplication` Enabled: a delete marker is replicated like a version.
    pub delete_markers: bool,
}

/// A bucket's whole replication configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Configuration {
    /// The `Role` the configuration names, kept as given.
    pub role: String,
    /// The rules, in the order given.
    pub rules: Vec<Rule>,
}

/// Why a configuration is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ReplicationError {
    /// No rule at all.
    #[error("a replication configuration holds at least one rule")]
    NoRules,
    /// More than [`MAX_RULES`].
    #[error("a replication configuration holds at most 1000 rules")]
    TooManyRules,
    /// No `Role`.
    #[error("a replication configuration names its Role")]
    NoRole,
    /// An ID longer than [`MAX_ID_CHARS`].
    #[error("a rule ID is at most 255 characters")]
    IdTooLong,
    /// Two rules with one ID.
    #[error("rule IDs must be unique")]
    DuplicateId,
    /// Two rules with one priority.
    #[error("rule priorities must be unique")]
    DuplicatePriority,
    /// A rule replicating the bucket into itself.
    #[error("the destination bucket cannot be the source bucket")]
    SameBucket,
}

/// The bucket an S3 bucket ARN (`arn:aws:s3:::<bucket>`) names, or `None` for anything else.
#[must_use]
pub fn destination_bucket(arn: &str) -> Option<&str> {
    arn.strip_prefix("arn:aws:s3:::")
        .filter(|name| !name.is_empty() && !name.contains('/'))
}

/// Checks `configuration` of the bucket `source` as S3 does before it is stored.
///
/// # Errors
/// The first [`ReplicationError`] found.
pub fn validate(configuration: &Configuration, source: &str) -> Result<(), ReplicationError> {
    let rules = &configuration.rules;
    if rules.is_empty() {
        return Err(ReplicationError::NoRules);
    }
    if rules.len() > MAX_RULES {
        return Err(ReplicationError::TooManyRules);
    }
    if configuration.role.is_empty() {
        return Err(ReplicationError::NoRole);
    }
    let mut ids = std::collections::BTreeSet::new();
    let mut priorities = std::collections::BTreeSet::new();
    for rule in rules {
        if rule.id.chars().count() > MAX_ID_CHARS {
            return Err(ReplicationError::IdTooLong);
        }
        if !rule.id.is_empty() && !ids.insert(rule.id.as_str()) {
            return Err(ReplicationError::DuplicateId);
        }
        if !priorities.insert(rule.priority) {
            return Err(ReplicationError::DuplicatePriority);
        }
        if rule.destination == source {
            return Err(ReplicationError::SameBucket);
        }
    }
    Ok(())
}

/// The rule that replicates `key`: the enabled rule of highest priority whose prefix the key has.
#[must_use]
pub fn matching<'a>(rules: &'a [Rule], key: &str) -> Option<&'a Rule> {
    rules
        .iter()
        .filter(|rule| rule.enabled && key.starts_with(&rule.prefix))
        .max_by_key(|rule| rule.priority)
}

#[cfg(test)]
#[path = "replication_tests.rs"]
mod tests;
