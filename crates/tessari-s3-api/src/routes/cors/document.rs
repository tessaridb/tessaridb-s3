//! Reading a `CORSConfiguration` document into rules, through the same bounded element tree the other configuration
//! documents use (no DTD, no entities beyond the predefined five, a depth the shape allows).

use tessari_s3_core::cors::{Method, Rule};
use tessari_s3_types::ErrorCode;

use super::super::xml_tree::{Node, malformed, single, tree};
use crate::Error;

/// Configuration › rule › element.
const MAX_DEPTH: usize = 3;

fn rule(node: &Node) -> Result<Rule, Error> {
    let (mut id, mut max_age) = (None, None);
    let mut found = Rule {
        id: None,
        origins: Vec::new(),
        methods: Vec::new(),
        headers: Vec::new(),
        expose: Vec::new(),
        max_age: None,
    };
    for child in &node.children {
        let text = child.text.trim().to_owned();
        match child.name.as_str() {
            "ID" => single(&mut id, child)?,
            "AllowedOrigin" => found.origins.push(text),
            "AllowedHeader" => found.headers.push(text),
            "ExposeHeader" => found.expose.push(text),
            "AllowedMethod" => found.methods.push(Method::parse(&text).ok_or_else(|| {
                Error::new(
                    ErrorCode::InvalidRequest,
                    format!("Found unsupported HTTP method in CORS config: {text}"),
                )
            })?),
            "MaxAgeSeconds" => single(&mut max_age, child)?,
            _ => return Err(malformed()),
        }
    }
    found.id = id.map(|node| node.text.trim().to_owned());
    found.max_age = max_age
        .map(|node| node.text.trim().parse::<u32>().map_err(|_| malformed()))
        .transpose()?;
    Ok(found)
}

/// The rules `document` lists, in order.
pub(super) fn parse(document: &[u8]) -> Result<Vec<Rule>, Error> {
    let root = tree(document, MAX_DEPTH)?;
    if root.name != "CORSConfiguration" {
        return Err(malformed());
    }
    root.children
        .iter()
        .map(|child| {
            if child.name == "CORSRule" {
                rule(child)
            } else {
                Err(malformed())
            }
        })
        .collect()
}
