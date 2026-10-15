//! Reading a `ReplicationConfiguration` document into a configuration. Only the `Filter` form of a rule is read; what
//! this server does not do — tag filters, existing-object replication, encryption, storage classes other than
//! STANDARD, time control, metrics, another account — is refused rather than accepted and ignored.

use tessari_s3_core::replication::{Configuration, Rule, destination_bucket};
use tessari_s3_types::ErrorCode;

use super::super::xml_tree::{Node, malformed, single, tree};
use crate::Error;

/// configuration › rule › filter › and › tag › key.
const MAX_DEPTH: usize = 6;

fn not_offered(what: &str) -> Error {
    Error::new(
        ErrorCode::NotImplemented,
        format!("{what} in a replication rule is not supported"),
    )
}

fn invalid(message: &str) -> Error {
    Error::new(ErrorCode::InvalidRequest, message)
}

fn status(node: Option<&Node>) -> Result<bool, Error> {
    let mut found = None;
    for child in node
        .map(|node| node.children.as_slice())
        .unwrap_or_default()
    {
        match child.name.as_str() {
            "Status" => single(&mut found, child)?,
            _ => return Err(malformed()),
        }
    }
    match found.map(|node| node.text.trim()) {
        Some("Enabled") => Ok(true),
        Some("Disabled") => Ok(false),
        _ => Err(malformed()),
    }
}

fn filter(node: &Node) -> Result<String, Error> {
    let mut prefix = None;
    for child in &node.children {
        match child.name.as_str() {
            "Prefix" => single(&mut prefix, child)?,
            "Tag" | "And" => return Err(not_offered("a tag filter")),
            _ => return Err(malformed()),
        }
    }
    Ok(prefix.map(|node| node.text.clone()).unwrap_or_default())
}

fn destination(node: &Node) -> Result<String, Error> {
    let mut bucket = None;
    for child in &node.children {
        match child.name.as_str() {
            "Bucket" => single(&mut bucket, child)?,
            "StorageClass" if child.text.trim() == "STANDARD" => {}
            "StorageClass" => return Err(not_offered("a storage class other than STANDARD")),
            "Account" | "AccessControlTranslation" => {
                return Err(not_offered("another account's destination"));
            }
            "EncryptionConfiguration" => return Err(not_offered("replica encryption")),
            "ReplicationTime" | "Metrics" => return Err(not_offered("replication time control")),
            _ => return Err(malformed()),
        }
    }
    let arn = bucket.ok_or_else(malformed)?.text.trim();
    destination_bucket(arn).map(str::to_owned).ok_or_else(|| {
        invalid("The destination bucket must be named by its ARN, arn:aws:s3:::<bucket>")
    })
}

fn rule(node: &Node) -> Result<Rule, Error> {
    let (mut id, mut priority, mut state, mut filtered, mut markers, mut target) =
        (None, None, None, None, None, None);
    for child in &node.children {
        match child.name.as_str() {
            "ID" => single(&mut id, child)?,
            "Priority" => single(&mut priority, child)?,
            "Status" => single(&mut state, child)?,
            "Filter" => single(&mut filtered, child)?,
            "DeleteMarkerReplication" => single(&mut markers, child)?,
            "Destination" => single(&mut target, child)?,
            "Prefix" => return Err(not_offered("the Prefix form (use Filter)")),
            "ExistingObjectReplication" => return Err(not_offered("existing-object replication")),
            "SourceSelectionCriteria" => return Err(not_offered("source selection")),
            _ => return Err(malformed()),
        }
    }
    let enabled = match state.map(|node| node.text.trim()) {
        Some("Enabled") => true,
        Some("Disabled") => false,
        _ => return Err(malformed()),
    };
    let priority = match priority.map(|node| node.text.trim()) {
        None => 0,
        Some(text) if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) => {
            text.parse().map_err(|_| malformed())?
        }
        Some(_) => return Err(malformed()),
    };
    let Some(markers) = markers else {
        return Err(invalid(
            "DeleteMarkerReplication must be specified for this version of the replication configuration",
        ));
    };
    Ok(Rule {
        id: id.map(|node| node.text.clone()).unwrap_or_default(),
        enabled,
        priority,
        prefix: filtered.map(filter).transpose()?.unwrap_or_default(),
        destination: destination(target.ok_or_else(malformed)?)?,
        delete_markers: status(Some(markers))?,
    })
}

/// The configuration `document` holds; validation of its values is left to the caller.
///
/// # Errors
/// `MalformedXML` for a document that is not the shape; `NotImplemented` for what this server does not offer;
/// `InvalidRequest` for a destination not named by its bucket ARN or a rule without `DeleteMarkerReplication`.
pub(super) fn parse(document: &[u8]) -> Result<Configuration, Error> {
    let root = tree(document, MAX_DEPTH)?;
    if root.name != "ReplicationConfiguration" {
        return Err(malformed());
    }
    let mut role = None;
    let mut rules = Vec::new();
    for child in &root.children {
        match child.name.as_str() {
            "Role" => single(&mut role, child)?,
            "Rule" => rules.push(rule(child)?),
            _ => return Err(malformed()),
        }
    }
    Ok(Configuration {
        role: role
            .map(|node| node.text.trim().to_owned())
            .unwrap_or_default(),
        rules,
    })
}
