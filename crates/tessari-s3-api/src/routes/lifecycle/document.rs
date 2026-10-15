//! Reading a `LifecycleConfiguration` document into rules. The document is read into a small element tree first —
//! names by local name, so the S3 namespace is optional; a DTD, an entity other than the five predefined ones, nesting
//! deeper than the shape allows, or text XML 1.0 cannot carry is malformed — and the tree is then read as the shape.

use tessari_s3_core::lifecycle::{Expiration, Noncurrent, Rule};
use tessari_s3_types::{ErrorCode, Timestamp};

use super::super::xml_tree::{Node, malformed, single, tree};
use crate::Error;

/// The deepest element the documented shape has — configuration › rule › filter › and › tag › key — so a filter this
/// server refuses is still read far enough to be named.
const MAX_DEPTH: usize = 6;

fn not_offered(what: &str) -> Error {
    Error::new(
        ErrorCode::NotImplemented,
        format!("{what} in a lifecycle rule is not supported"),
    )
}

fn whole(node: &Node) -> Result<u32, Error> {
    let text = node.text.trim();
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(malformed());
    }
    text.parse().map_err(|_| malformed())
}

fn filter(node: &Node) -> Result<String, Error> {
    let mut prefix = None;
    for child in &node.children {
        match child.name.as_str() {
            "Prefix" => single(&mut prefix, child)?,
            "Tag" | "And" => return Err(not_offered("a tag filter")),
            "ObjectSizeGreaterThan" | "ObjectSizeLessThan" => {
                return Err(not_offered("an object size filter"));
            }
            _ => return Err(malformed()),
        }
    }
    Ok(prefix.map(|node| node.text.clone()).unwrap_or_default())
}

fn expiration(node: &Node) -> Result<(Option<Expiration>, bool), Error> {
    let (mut days, mut date, mut marker) = (None, None, None);
    for child in &node.children {
        match child.name.as_str() {
            "Days" => single(&mut days, child)?,
            "Date" => single(&mut date, child)?,
            "ExpiredObjectDeleteMarker" => single(&mut marker, child)?,
            _ => return Err(malformed()),
        }
    }
    let marker = match marker.map(|node| node.text.trim()) {
        None | Some("false") => false,
        Some("true") => true,
        Some(_) => return Err(malformed()),
    };
    let when = match (days, date) {
        (Some(_), Some(_)) => return Err(malformed()),
        (Some(days), None) => Some(Expiration::Days(whole(days)?)),
        (None, Some(date)) => {
            let at = Timestamp::parse_iso8601(&date.text).ok_or_else(malformed)?;
            if at.seconds.rem_euclid(86_400) != 0 || at.nanos != 0 {
                return Err(Error::new(
                    ErrorCode::InvalidArgument,
                    "'Date' must be at midnight GMT",
                ));
            }
            Some(Expiration::Date(at))
        }
        (None, None) => None,
    };
    if when.is_none() && !marker {
        return Err(malformed());
    }
    Ok((when, marker))
}

fn noncurrent(node: &Node) -> Result<Noncurrent, Error> {
    let (mut days, mut keep) = (None, None);
    for child in &node.children {
        match child.name.as_str() {
            "NoncurrentDays" => single(&mut days, child)?,
            "NewerNoncurrentVersions" => single(&mut keep, child)?,
            _ => return Err(malformed()),
        }
    }
    Ok(Noncurrent {
        days: whole(days.ok_or_else(malformed)?)?,
        keep: keep.map(whole).transpose()?,
    })
}

fn abort_days(node: &Node) -> Result<u32, Error> {
    match node.children.as_slice() {
        [days] if days.name == "DaysAfterInitiation" => whole(days),
        _ => Err(malformed()),
    }
}

fn rule(node: &Node) -> Result<Rule, Error> {
    let (mut id, mut status, mut filtered, mut legacy) = (None, None, None, None);
    let mut found = Rule {
        id: String::new(),
        enabled: false,
        prefix: String::new(),
        expiration: None,
        expired_object_delete_marker: false,
        noncurrent: None,
        abort_after_days: None,
    };
    let (mut expiry, mut older, mut abort) = (None, None, None);
    for child in &node.children {
        match child.name.as_str() {
            "ID" => single(&mut id, child)?,
            "Status" => single(&mut status, child)?,
            "Filter" => single(&mut filtered, child)?,
            "Prefix" => single(&mut legacy, child)?,
            "Expiration" => single(&mut expiry, child)?,
            "NoncurrentVersionExpiration" => single(&mut older, child)?,
            "AbortIncompleteMultipartUpload" => single(&mut abort, child)?,
            "Transition" | "NoncurrentVersionTransition" => {
                return Err(not_offered("a transition"));
            }
            _ => return Err(malformed()),
        }
    }
    found.enabled = match status.map(|node| node.text.trim()) {
        Some("Enabled") => true,
        Some("Disabled") => false,
        _ => return Err(malformed()),
    };
    found.id = id.map(|node| node.text.clone()).unwrap_or_default();
    found.prefix = match (filtered, legacy) {
        (Some(_), Some(_)) => return Err(malformed()),
        (Some(node), None) => filter(node)?,
        (None, Some(node)) => node.text.clone(),
        (None, None) => String::new(),
    };
    if let Some(node) = expiry {
        (found.expiration, found.expired_object_delete_marker) = expiration(node)?;
    }
    found.noncurrent = older.map(noncurrent).transpose()?;
    found.abort_after_days = abort.map(abort_days).transpose()?;
    Ok(found)
}

/// The rules `document` holds, in document order; validation of their values is left to the caller.
///
/// # Errors
/// `MalformedXML` for a document that is not the shape; `NotImplemented` for a transition or a tag or size filter;
/// `InvalidArgument` for an expiration date that is not at midnight UTC.
pub(super) fn parse(document: &[u8]) -> Result<Vec<Rule>, Error> {
    let root = tree(document, MAX_DEPTH)?;
    if root.name != "LifecycleConfiguration" {
        return Err(malformed());
    }
    root.children
        .iter()
        .map(|child| {
            if child.name == "Rule" {
                rule(child)
            } else {
                Err(malformed())
            }
        })
        .collect()
}
