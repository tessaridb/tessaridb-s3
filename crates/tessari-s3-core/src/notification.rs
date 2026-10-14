//! Bucket event notification configurations (S3 ref 11 M5): which changes a bucket reports, to which operator-declared
//! destination, for which keys, and the checks a configuration passes before it is stored.

/// Longest configuration ID, in characters.
pub const MAX_ID_CHARS: usize = 255;

/// One rule: a `QueueConfiguration`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    /// The rule's ID.
    pub id: String,
    /// The id of the operator-declared destination its ARN names.
    pub target: String,
    /// The event names it reports, wildcards included.
    pub events: Vec<String>,
    /// Only keys starting with this; empty for every key.
    pub prefix: String,
    /// Only keys ending with this; empty for every key.
    pub suffix: String,
}

/// The prefix of this server's destination ARNs; the target id follows it.
pub const ARN_PREFIX: &str = "arn:tessaridb:sqs:::";

/// The event names a rule may report: the changes this server records, and their families' wildcards.
pub const EVENTS: &[&str] = &[
    "s3:ObjectCreated:*",
    "s3:ObjectCreated:Put",
    "s3:ObjectCreated:Copy",
    "s3:ObjectCreated:CompleteMultipartUpload",
    "s3:ObjectRemoved:*",
    "s3:ObjectRemoved:Delete",
    "s3:ObjectRemoved:DeleteMarkerCreated",
    "s3:LifecycleExpiration:*",
    "s3:LifecycleExpiration:Delete",
    "s3:LifecycleExpiration:DeleteMarkerCreated",
];

/// Why a configuration is refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NotificationError {
    /// A rule with no event.
    #[error("a notification configuration names at least one event")]
    NoEvent,
    /// An event this server does not record.
    #[error("The event is not supported for notification: {0}")]
    UnsupportedEvent(String),
    /// A destination the operator did not declare.
    #[error("Unable to validate the following destination configurations: {0}")]
    UndeclaredTarget(String),
    /// Two rules with one ID.
    #[error("notification configuration IDs must be unique")]
    DuplicateId,
    /// An ID longer than [`MAX_ID_CHARS`].
    #[error("a notification configuration ID is at most 255 characters")]
    IdTooLong,
    /// Two rules reporting one event for keys both may match.
    #[error(
        "Configurations overlap. Configurations on the same bucket cannot share a common event type."
    )]
    Overlap,
}

/// The ARN a configuration names `target` by.
#[must_use]
pub fn arn(target: &str) -> String {
    format!("{ARN_PREFIX}{target}")
}

/// The target id `arn` names, when it is this server's ARN.
#[must_use]
pub fn target_of(arn: &str) -> Option<&str> {
    arn.strip_prefix(ARN_PREFIX)
        .filter(|target| !target.is_empty())
}

/// Whether one rule's event and another's can name the same change: equal, or one is the other's family wildcard.
fn events_meet(one: &str, other: &str) -> bool {
    let family = |event: &str| event.rsplit_once(':').map(|(family, _)| family.to_owned());
    one == other
        || (one.ends_with(":*") && family(one) == family(other))
        || (other.ends_with(":*") && family(one) == family(other))
}

fn overlap(one: &Rule, other: &Rule) -> bool {
    let events = one
        .events
        .iter()
        .any(|event| other.events.iter().any(|theirs| events_meet(event, theirs)));
    let prefixes = one.prefix.starts_with(&other.prefix) || other.prefix.starts_with(&one.prefix);
    let suffixes = one.suffix.ends_with(&other.suffix) || other.suffix.ends_with(&one.suffix);
    events && prefixes && suffixes
}

/// Checks `rules` before they are stored; `declared` answers whether a target id is one the operator declared. An
/// empty configuration is valid: it turns notifications off.
///
/// # Errors
/// The first [`NotificationError`] found.
pub fn validate(rules: &[Rule], declared: impl Fn(&str) -> bool) -> Result<(), NotificationError> {
    let mut ids = std::collections::BTreeSet::new();
    for (index, rule) in rules.iter().enumerate() {
        if rule.id.chars().count() > MAX_ID_CHARS {
            return Err(NotificationError::IdTooLong);
        }
        if !ids.insert(rule.id.as_str()) {
            return Err(NotificationError::DuplicateId);
        }
        if rule.events.is_empty() {
            return Err(NotificationError::NoEvent);
        }
        if let Some(event) = rule
            .events
            .iter()
            .find(|event| !EVENTS.contains(&event.as_str()))
        {
            return Err(NotificationError::UnsupportedEvent(event.clone()));
        }
        if !declared(&rule.target) {
            return Err(NotificationError::UndeclaredTarget(arn(&rule.target)));
        }
        if rules
            .iter()
            .take(index)
            .any(|earlier| overlap(earlier, rule))
        {
            return Err(NotificationError::Overlap);
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "notification_tests.rs"]
mod tests;
