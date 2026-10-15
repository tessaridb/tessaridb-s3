//! DeleteObjects: up to 1,000 keys in one request, each removed on its own and reported on its own. The body must
//! carry a Content-MD5 or a checksum and is checked against it before anything is removed; a request naming what
//! this server does not implement (a per-object condition) is refused whole, so it deletes nothing. An entry naming a
//! version removes that version, and only for who administers the bucket.

use axum::body::Body;
use axum::http::{Response, StatusCode};
use quick_xml::Reader;
use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::Event;
use tessari_s3_constants::{DELETE_OBJECTS_BODY_MAX, DELETE_OBJECTS_MAX_KEYS};
use tessari_s3_core::objects::checksum::Hashes;
use tessari_s3_storage::objects::{Deleted, VersionRemoved};
use tessari_s3_types::{ErrorCode, ObjectKey};

use super::headers::read_integrity;
use super::put::check;
use crate::pipeline::authorize::may_configure;
use crate::pipeline::body::read_verified;
use crate::pipeline::call::Call;
use crate::pipeline::response::xml_response;
use crate::xml::{S3_NAMESPACE, escape, xml_text};
use crate::{Error, Result};

/// What the request asks for.
#[derive(Debug, PartialEq, Eq)]
struct Request {
    quiet: bool,
    /// Each key, with the version the entry names when it names one.
    entries: Vec<(ObjectKey, Option<String>)>,
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
    Version,
}

/// Reads the `Delete` document. Names compare by local name, so the S3 namespace is optional; a DTD, an entity other
/// than the five predefined ones, or a character XML 1.0 cannot carry is malformed.
fn parse(document: &[u8]) -> Result<Request> {
    let mut reader = Reader::from_reader(document);
    let mut at = At::Outside;
    let mut text = String::new();
    let mut key: Option<String> = None;
    let mut version: Option<String> = None;
    let mut quiet = false;
    let mut entries = Vec::new();
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
                    (At::Object, "VersionId") if version.is_none() => At::Version,
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
            Event::Text(content) if matches!(at, At::Key | At::Quiet | At::Version) => {
                text.push_str(&content.xml10_content());
            }
            Event::CData(content) if matches!(at, At::Key | At::Version) => {
                text.push_str(&content.xml10_content());
            }
            Event::GeneralRef(reference) if matches!(at, At::Key | At::Quiet | At::Version) => {
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
                    // Read back into the answer, so it must be text XML can carry.
                    At::Version => {
                        let named = std::mem::take(&mut text);
                        if xml_text(&named).is_none() {
                            return Err(malformed());
                        }
                        version = Some(named);
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
                        entries.push((
                            ObjectKey::new(&named).map_err(|_| malformed())?,
                            version.take(),
                        ));
                        if entries.len() > DELETE_OBJECTS_MAX_KEYS {
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
    if !closed || entries.is_empty() {
        return Err(malformed());
    }
    Ok(Request { quiet, entries })
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
    let bucket = call.bucket()?;
    // Sent to the bucket's region once the body is known to be well formed, so a malformed one never reads the store.
    crate::routes::buckets::resident(call).await?;
    // An entry naming a version deletes it for good, which needs who administers the bucket; without that, those
    // entries are refused one by one and the rest go ahead (S3 reports each key on its own).
    let versions_allowed = !request.entries.iter().any(|(_, version)| version.is_some())
        || may_configure(call.state, call.principal, bucket).await?;
    let (allowed, refused): (Vec<_>, Vec<_>) = request
        .entries
        .iter()
        .cloned()
        .enumerate()
        .partition(|(_, (_, version))| versions_allowed || version.is_none());
    let (positions, entries): (Vec<usize>, Vec<_>) = allowed.into_iter().unzip();
    let outcomes = call
        .state
        .storage()
        .objects()
        .delete_many(bucket, &entries)
        .await?
        .ok_or_else(|| {
            Error::new(
                ErrorCode::NoSuchBucket,
                "the specified bucket does not exist",
            )
        })?;
    let mut answers: Vec<Option<std::result::Result<Deleted, Error>>> =
        request.entries.iter().map(|_| None).collect();
    for (position, outcome) in positions.into_iter().zip(outcomes) {
        if let Some(answer) = answers.get_mut(position) {
            *answer = Some(outcome.map_err(Error::from));
        }
    }
    for (position, _) in refused {
        if let Some(answer) = answers.get_mut(position) {
            *answer = Some(Err(Error::new(ErrorCode::AccessDenied, "Access Denied")));
        }
    }
    let mut out = format!("<DeleteResult xmlns=\"{S3_NAMESPACE}\">");
    for ((key, version), answer) in request.entries.iter().zip(answers) {
        if let Some(answer) = answer {
            report(&mut out, key, version.as_deref(), answer, request.quiet);
        }
    }
    out.push_str("</DeleteResult>");
    Ok(xml_response(StatusCode::OK, out))
}

/// One entry's `<Deleted>` or `<Error>`; a quiet request reports only errors.
fn report(
    out: &mut String,
    key: &ObjectKey,
    version: Option<&str>,
    answer: std::result::Result<Deleted, Error>,
    quiet: bool,
) {
    // Every key and version passed the XML check in `parse`, so each has a text form.
    let key = xml_text(key.as_str()).unwrap_or_default();
    let named = version
        .and_then(xml_text)
        .map(|id| format!("<VersionId>{id}</VersionId>"))
        .unwrap_or_default();
    let error = |out: &mut String, error: Error| {
        out.push_str(&format!(
            "<Error><Key>{key}</Key>{named}<Code>{}</Code><Message>{}</Message></Error>",
            error.code.as_str(),
            escape(&error.message)
        ));
    };
    let marker = |id: &str| {
        format!(
            "<DeleteMarker>true</DeleteMarker><DeleteMarkerVersionId>{}</DeleteMarkerVersionId>",
            xml_text(id).unwrap_or_default()
        )
    };
    match answer {
        Err(refused) => error(out, refused),
        Ok(Deleted::Version(VersionRemoved::Malformed)) => error(
            out,
            Error::new(ErrorCode::InvalidArgument, "Invalid version id specified"),
        ),
        Ok(Deleted::Version(VersionRemoved::NoSuchBucket)) => error(
            out,
            Error::new(
                ErrorCode::NoSuchBucket,
                "the specified bucket does not exist",
            ),
        ),
        Ok(_) if quiet => {}
        Ok(Deleted::Current { marker: added }) => {
            let added = added.as_deref().map(marker).unwrap_or_default();
            out.push_str(&format!("<Deleted><Key>{key}</Key>{added}</Deleted>"));
        }
        Ok(Deleted::Version(VersionRemoved::Done { marker: true })) => {
            let removed = version.map(marker).unwrap_or_default();
            out.push_str(&format!(
                "<Deleted><Key>{key}</Key>{named}{removed}</Deleted>"
            ));
        }
        Ok(Deleted::Version(VersionRemoved::Done { marker: false } | VersionRemoved::Absent)) => {
            out.push_str(&format!("<Deleted><Key>{key}</Key>{named}</Deleted>"));
        }
    }
}

#[cfg(test)]
#[path = "delete_many_tests.rs"]
mod tests;
