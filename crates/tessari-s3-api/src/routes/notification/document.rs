//! Reading a `NotificationConfiguration` document into rules. Only `QueueConfiguration` is offered, naming an
//! operator-declared destination by its ARN; topic, function and event-bus configurations are refused rather than
//! accepted and ignored.

use tessari_s3_core::notification::{Rule, target_of};
use tessari_s3_types::ErrorCode;

use super::super::xml_tree::{Node, malformed, single, tree};
use crate::Error;

/// The deepest element the documented shape has — configuration › queue › filter › key › rule › name.
const MAX_DEPTH: usize = 6;

fn not_offered(what: &str) -> Error {
    Error::new(
        ErrorCode::NotImplemented,
        format!(
            "{what} is not supported; a notification names a destination with QueueConfiguration"
        ),
    )
}

fn invalid(message: &str) -> Error {
    Error::new(ErrorCode::InvalidArgument, message.to_owned())
}

/// The prefix and suffix a `Filter` names; each at most once.
fn filter(node: &Node) -> Result<(String, String), Error> {
    let (mut prefix, mut suffix) = (None, None);
    for key in &node.children {
        if key.name != "S3Key" {
            return Err(malformed());
        }
        for rule in &key.children {
            if rule.name != "FilterRule" {
                return Err(malformed());
            }
            let (mut name, mut value) = (None, None);
            for part in &rule.children {
                match part.name.as_str() {
                    "Name" => single(&mut name, part)?,
                    "Value" => single(&mut value, part)?,
                    _ => return Err(malformed()),
                }
            }
            let (Some(name), Some(value)) = (name, value) else {
                return Err(malformed());
            };
            let slot = match name.text.trim().to_ascii_lowercase().as_str() {
                "prefix" => &mut prefix,
                "suffix" => &mut suffix,
                _ => {
                    return Err(invalid(
                        "A filter rule name must be either prefix or suffix",
                    ));
                }
            };
            if slot.replace(value.text.clone()).is_some() {
                return Err(invalid(
                    "Cannot specify more than one prefix or suffix rule in a filter",
                ));
            }
        }
    }
    Ok((prefix.unwrap_or_default(), suffix.unwrap_or_default()))
}

/// One `QueueConfiguration`; an `Id` left out is `None`, for the caller to give one.
fn rule(node: &Node) -> Result<(Option<String>, Rule), Error> {
    let (mut id, mut queue, mut found) = (None, None, None);
    let mut events = Vec::new();
    for child in &node.children {
        match child.name.as_str() {
            "Id" => single(&mut id, child)?,
            "Queue" => single(&mut queue, child)?,
            "Event" => events.push(child.text.trim().to_owned()),
            "Filter" => single(&mut found, child)?,
            _ => return Err(malformed()),
        }
    }
    let Some(queue) = queue else {
        return Err(malformed());
    };
    if events.is_empty() {
        return Err(malformed());
    }
    let arn = queue.text.trim();
    let target = target_of(arn).ok_or_else(|| {
        invalid(&format!(
            "Unable to validate the following destination configurations: {arn}"
        ))
    })?;
    let (prefix, suffix) = found.map(filter).transpose()?.unwrap_or_default();
    Ok((
        id.map(|id| id.text.trim().to_owned()),
        Rule {
            id: String::new(),
            target: target.to_owned(),
            events,
            prefix,
            suffix,
        },
    ))
}

/// The rules `document` holds, each with the `Id` it was sent with, if any.
pub(super) fn parse(document: &[u8]) -> Result<Vec<(Option<String>, Rule)>, Error> {
    let root = tree(document, MAX_DEPTH)?;
    if root.name != "NotificationConfiguration" {
        return Err(malformed());
    }
    root.children
        .iter()
        .map(|child| match child.name.as_str() {
            "QueueConfiguration" => rule(child),
            "TopicConfiguration" => Err(not_offered("TopicConfiguration")),
            "CloudFunctionConfiguration" => Err(not_offered("CloudFunctionConfiguration")),
            "EventBridgeConfiguration" => Err(not_offered("EventBridgeConfiguration")),
            _ => Err(malformed()),
        })
        .collect()
}
