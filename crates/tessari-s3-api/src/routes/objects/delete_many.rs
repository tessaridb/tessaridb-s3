//! DeleteObjects: up to 1,000 keys in one request, each removed on its own and reported on its own. The body must
//! carry a Content-MD5 or a checksum and is checked against it before anything is removed; a request naming what
//! this server does not implement (a version, a per-object condition) is refused whole, so it deletes nothing.

use axum::body::Body;
use axum::http::{Response, StatusCode};
use quick_xml::Reader;
use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::Event;
use tessari_s3_constants::{DELETE_OBJECTS_BODY_MAX, DELETE_OBJECTS_MAX_KEYS};
use tessari_s3_core::objects::checksum::Hashes;
use tessari_s3_types::{ErrorCode, ObjectKey};

use super::headers::read_integrity;
use super::put::check;
use crate::pipeline::body::read_verified;
use crate::pipeline::call::Call;
use crate::pipeline::response::xml_response;
use crate::xml::{S3_NAMESPACE, escape, xml_text};
use crate::{Error, Result};

/// What the request asks for.
#[derive(Debug, PartialEq, Eq)]
struct Request {
    quiet: bool,
    keys: Vec<ObjectKey>,
}

fn malformed() -> Error {
    Error::new(
        ErrorCode::MalformedXml,
        "the XML body is not well formed or not the expected shape",
    )
}

/// Where the reader is in the `Delete` document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum At {
    Outside,
    Delete,
    Quiet,
    Object,
    Key,
}

/// Reads the `Delete` document. Names compare by local name, so the S3 namespace is optional; a DTD, an entity other
/// than the five predefined ones, or a character XML 1.0 cannot carry is malformed.
fn parse(document: &[u8]) -> Result<Request> {
    let mut reader = Reader::from_reader(document);
    let mut at = At::Outside;
    let mut text = String::new();
    let mut key: Option<String> = None;
    let mut quiet = false;
    let mut keys = Vec::new();
    let mut closed = false;
    loop {
        match reader.read_event().map_err(|_| malformed())? {
            Event::DocType(_) => return Err(malformed()),
            Event::Start(start) => {
                at = match (at, start.local_name().as_ref()) {
                    (At::Outside, "Delete") if !closed => At::Delete,
                    (At::Delete, "Quiet") => At::Quiet,
                    (At::Delete, "Object") => At::Object,
                    (At::Object, "Key") => At::Key,
                    (At::Object, "VersionId") => {
                        return Err(Error::new(
                            ErrorCode::NotImplemented,
                            "deleting a version is not implemented",
                        ));
                    }
                    (At::Object, "ETag" | "LastModifiedTime" | "Size") => {
                        return Err(Error::new(
                            ErrorCode::NotImplemented,
                            "conditional deletes are not implemented",
                        ));
                    }
                    _ => return Err(malformed()),
                };
                text.clear();
            }
            Event::Empty(_) => return Err(malformed()),
            Event::Text(content) if matches!(at, At::Key | At::Quiet) => {
                text.push_str(&content.xml10_content());
            }
            Event::CData(content) if at == At::Key => {
                text.push_str(&content.xml10_content());
            }
            Event::GeneralRef(reference) if matches!(at, At::Key | At::Quiet) => {
                let resolved = match reference.resolve_char_ref().map_err(|_| malformed())? {
                    Some(character) => character,
                    None => resolve_predefined_entity(&reference.xml10_content())
                        .and_then(|entity| entity.chars().next())
                        .ok_or_else(malformed)?,
                };
                text.push(resolved);
            }
            Event::Text(content) => {
                if !content.xml10_content().trim().is_empty() {
                    return Err(malformed());
                }
            }
            Event::End(_) => {
                at = match at {
                    At::Key => {
                        key = Some(std::mem::take(&mut text));
                        At::Object
                    }
                    At::Quiet => {
                        quiet = match text.trim() {
                            "true" => true,
                            "false" => false,
                            _ => return Err(malformed()),
                        };
                        At::Delete
                    }
                    At::Object => {
                        let named = key.take().ok_or_else(malformed)?;
                        if xml_text(&named).is_none() {
                            return Err(malformed());
                        }
                        keys.push(ObjectKey::new(&named).map_err(|_| malformed())?);
                        if keys.len() > DELETE_OBJECTS_MAX_KEYS {
                            return Err(malformed());
                        }
                        At::Delete
                    }
                    At::Delete => {
                        closed = true;
                        At::Outside
                    }
                    At::Outside => return Err(malformed()),
                };
            }
            Event::Eof => break,
            Event::Decl(_) | Event::Comment(_) | Event::PI(_) if at == At::Outside => {}
            _ => return Err(malformed()),
        }
    }
    if !closed || keys.is_empty() {
        return Err(malformed());
    }
    Ok(Request { quiet, keys })
}

/// `POST /{bucket}?delete`.
pub(crate) async fn delete_many(call: &Call<'_>, body: Body) -> Result<Response<Body>> {
    let integrity = read_integrity(call.headers)?;
    if integrity.content_md5.is_none() && integrity.checksum.is_none() {
        return Err(Error::new(
            ErrorCode::InvalidRequest,
            "Missing required header for this request: Content-MD5 or x-amz-checksum-*",
        ));
    }
    let document = read_verified(body, call.verified, DELETE_OBJECTS_BODY_MAX).await?;
    let mut hashes = Hashes::new();
    hashes.update(&document);
    check(&integrity, &hashes.finish(), None)?;
    let request = parse(&document)?;
    let outcomes = call
        .state
        .storage()
        .objects()
        .delete_many(call.bucket()?, &request.keys)
        .await?
        .ok_or_else(|| {
            Error::new(
                ErrorCode::NoSuchBucket,
                "the specified bucket does not exist",
            )
        })?;
    let mut out = format!("<DeleteResult xmlns=\"{S3_NAMESPACE}\">");
    for (key, outcome) in request.keys.iter().zip(outcomes) {
        // Every key passed the XML check in `parse`, so it has a text form.
        let key = xml_text(key.as_str()).unwrap_or_default();
        match outcome {
            Ok(()) if request.quiet => {}
            Ok(()) => out.push_str(&format!("<Deleted><Key>{key}</Key></Deleted>")),
            Err(error) => {
                let error = Error::from(error);
                out.push_str(&format!(
                    "<Error><Key>{key}</Key><Code>{}</Code><Message>{}</Message></Error>",
                    error.code.as_str(),
                    escape(&error.message)
                ));
            }
        }
    }
    out.push_str("</DeleteResult>");
    Ok(xml_response(StatusCode::OK, out))
}

#[cfg(test)]
#[path = "delete_many_tests.rs"]
mod tests;
