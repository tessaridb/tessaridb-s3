//! Reading an XML request document into a small element tree first — names by local name, so the S3 namespace is
//! optional; a DTD, an entity other than the five predefined ones, nesting deeper than the caller's shape allows, or
//! text XML 1.0 cannot carry is malformed — so a configuration parser then reads the tree as its shape.

use quick_xml::Reader;
use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::Event;
use tessari_s3_types::ErrorCode;

use crate::Error;

/// The answer to a document that is not well formed or not the shape its operation reads.
pub(super) fn malformed() -> Error {
    Error::new(
        ErrorCode::MalformedXml,
        "the XML body is not well formed or not the expected shape",
    )
}

/// One element: its local name, its text and its children.
pub(super) struct Node {
    pub(super) name: String,
    pub(super) text: String,
    pub(super) children: Vec<Node>,
}

/// The element tree of `document`, at most `max_depth` elements deep.
pub(super) fn tree(document: &[u8], max_depth: usize) -> Result<Node, Error> {
    let mut reader = Reader::from_reader(document);
    let mut open: Vec<Node> = Vec::new();
    loop {
        match reader.read_event().map_err(|_| malformed())? {
            Event::DocType(_) => return Err(malformed()),
            Event::Start(start) => {
                if open.len() >= max_depth {
                    return Err(malformed());
                }
                let name = start.local_name().as_ref().to_owned();
                open.push(Node {
                    name,
                    text: String::new(),
                    children: Vec::new(),
                });
            }
            Event::Empty(empty) => {
                let name = empty.local_name().as_ref().to_owned();
                let node = Node {
                    name,
                    text: String::new(),
                    children: Vec::new(),
                };
                match open.last_mut() {
                    Some(parent) => parent.children.push(node),
                    None => return Ok(node),
                }
            }
            Event::Text(content) => match open.last_mut() {
                Some(node) => node.text.push_str(&content.xml10_content()),
                None if content.xml10_content().trim().is_empty() => {}
                None => return Err(malformed()),
            },
            Event::CData(content) => {
                open.last_mut()
                    .ok_or_else(malformed)?
                    .text
                    .push_str(&content.xml10_content());
            }
            Event::GeneralRef(reference) => {
                let resolved = match reference.resolve_char_ref().map_err(|_| malformed())? {
                    Some(character) => character,
                    None => resolve_predefined_entity(&reference.xml10_content())
                        .and_then(|entity| entity.chars().next())
                        .ok_or_else(malformed)?,
                };
                open.last_mut().ok_or_else(malformed)?.text.push(resolved);
            }
            Event::End(_) => {
                let node = open.pop().ok_or_else(malformed)?;
                match open.last_mut() {
                    Some(parent) => parent.children.push(node),
                    None => return Ok(node),
                }
            }
            Event::Eof => return Err(malformed()),
            _ => {}
        }
    }
}

/// Records `node` as the one element of its kind; a second one is malformed.
pub(super) fn single<'a>(seen: &mut Option<&'a Node>, node: &'a Node) -> Result<(), Error> {
    if seen.replace(node).is_some() {
        return Err(malformed());
    }
    Ok(())
}
