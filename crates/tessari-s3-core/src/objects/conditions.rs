//! Conditional reads (RFC 9110 with S3's precedence): If-Match beats If-Unmodified-Since, If-None-Match beats
//! If-Modified-Since.

use tessari_s3_types::Timestamp;

/// The conditional headers of a GET or HEAD, as sent.
#[derive(Debug, Clone, Copy, Default)]
pub struct ReadConditions<'a> {
    /// `If-Match`.
    pub if_match: Option<&'a str>,
    /// `If-None-Match`.
    pub if_none_match: Option<&'a str>,
    /// `If-Modified-Since`, an HTTP date.
    pub if_modified_since: Option<&'a str>,
    /// `If-Unmodified-Since`, an HTTP date.
    pub if_unmodified_since: Option<&'a str>,
}

/// What the conditions decide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadVerdict {
    /// Serve the object.
    Proceed,
    /// 304 Not Modified.
    NotModified,
    /// 412 Precondition Failed.
    PreconditionFailed,
}

/// Evaluates `conditions` against the object's quoted `etag` and its last-modified instant.
#[must_use]
pub fn evaluate(conditions: &ReadConditions<'_>, etag: &str, modified: Timestamp) -> ReadVerdict {
    // HTTP dates have one-second resolution, so the stored instant is compared at whole seconds.
    let modified = modified.seconds;
    let date = |text: Option<&str>| text.and_then(Timestamp::parse_http_date).map(|t| t.seconds);
    if let Some(list) = conditions.if_match {
        if !etag_list_matches(list, etag) {
            return ReadVerdict::PreconditionFailed;
        }
    } else if date(conditions.if_unmodified_since).is_some_and(|since| modified > since) {
        return ReadVerdict::PreconditionFailed;
    }
    if let Some(list) = conditions.if_none_match {
        if etag_list_matches(list, etag) {
            return ReadVerdict::NotModified;
        }
    } else if date(conditions.if_modified_since).is_some_and(|since| modified <= since) {
        return ReadVerdict::NotModified;
    }
    ReadVerdict::Proceed
}

/// Whether an `If-Match` / `If-None-Match` list (`*`, or comma-separated entity tags, weak or strong) holds `etag`.
#[must_use]
pub fn etag_list_matches(list: &str, etag: &str) -> bool {
    let bare = |tag: &str| {
        tag.trim()
            .trim_start_matches("W/")
            .trim_matches('"')
            .to_owned()
    };
    let wanted = bare(etag);
    list.split(',')
        .any(|candidate| candidate.trim() == "*" || bare(candidate) == wanted)
}

#[cfg(test)]
#[path = "conditions_tests.rs"]
mod tests;
