//! CompleteMultipartUpload: the listed parts become the object in one commit, after every part is checked against
//! what was uploaded. The request finishes before the response starts — the commit writes metadata only, no part's
//! bytes are copied — so a failure is an ordinary error response, never an error inside a 200.

use axum::body::Body;
use axum::http::{Response, StatusCode};
use quick_xml::Reader;
use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::Event;
use tessari_s3_constants::{COMPLETE_MULTIPART_BODY_MAX, MULTIPART_MAX_PARTS};
use tessari_s3_core::objects::checksum::{ChecksumAlgorithm, Declared, Hashes};
use tessari_s3_core::objects::multipart::{ListedPart, Refusal};
use tessari_s3_storage::multipart::{Completed, Completion};
use tessari_s3_storage::objects::Written;
use tessari_s3_types::{ErrorCode, ObjectKey, PartNumber};

use super::{key_text, no_such_upload, upload_id};
use crate::pipeline::body::read_verified;
use crate::pipeline::call::Call;
use crate::pipeline::response::xml_response;
use crate::routes::objects::headers::read_put_headers;
use crate::routes::quota::Room;
use crate::xml::{S3_NAMESPACE, escape};
use crate::{Error, Result};

fn malformed() -> Error {
    Error::new(
        ErrorCode::MalformedXml,
        "the XML body is not well formed or not the expected shape",
    )
}

/// Where the reader is in the `CompleteMultipartUpload` document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum At {
    Outside,
    Document,
    Part,
    Number,
    ETag,
    Checksum(ChecksumAlgorithm),
}

/// One `Part` element being read.
#[derive(Debug, Default)]
struct Reading {
    number: Option<u16>,
    etag: Option<String>,
    checksums: Vec<(ChecksumAlgorithm, String)>,
}

/// What a `Part` child element names.
fn part_child(name: &str) -> Result<At> {
    match name {
        "PartNumber" => Ok(At::Number),
        "ETag" => Ok(At::ETag),
        _ => match name.strip_prefix("Checksum").map(ChecksumAlgorithm::parse) {
            Some(Some(Declared::Supported(algorithm))) => Ok(At::Checksum(algorithm)),
            Some(Some(Declared::Unsupported)) => Err(Error::new(
                ErrorCode::NotImplemented,
                format!("the checksum element {name} is not implemented"),
            )),
            _ => Err(malformed()),
        },
    }
}

/// Reads the `CompleteMultipartUpload` document: names by local name, a DTD or an unknown entity malformed, at
/// least one and at most 10,000 parts. The order is kept as listed; checking it is the storage's job.
fn parse(document: &[u8]) -> Result<Vec<ListedPart>> {
    let mut reader = Reader::from_reader(document);
    let mut at = At::Outside;
    let mut text = String::new();
    let mut part = Reading::default();
    let mut parts = Vec::new();
    let mut closed = false;
    loop {
        match reader.read_event().map_err(|_| malformed())? {
            Event::DocType(_) => return Err(malformed()),
            Event::Start(start) => {
                at = match (at, start.local_name().as_ref()) {
                    (At::Outside, "CompleteMultipartUpload") if !closed => At::Document,
                    (At::Document, "Part") => At::Part,
                    (At::Part, name) => part_child(name)?,
                    _ => return Err(malformed()),
                };
                text.clear();
            }
            Event::Text(content) if matches!(at, At::Number | At::ETag | At::Checksum(_)) => {
                text.push_str(&content.xml10_content());
            }
            Event::GeneralRef(reference)
                if matches!(at, At::Number | At::ETag | At::Checksum(_)) =>
            {
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
                    At::Number => {
                        let number = PartNumber::parse(text.trim()).map_err(|_| malformed())?;
                        part.number = Some(number.get());
                        At::Part
                    }
                    At::ETag => {
                        part.etag = Some(std::mem::take(&mut text));
                        At::Part
                    }
                    At::Checksum(algorithm) => {
                        part.checksums.push((algorithm, text.trim().to_owned()));
                        At::Part
                    }
                    At::Part => {
                        let done = std::mem::take(&mut part);
                        parts.push(ListedPart {
                            number: done.number.ok_or_else(malformed)?,
                            etag: done.etag.ok_or_else(malformed)?,
                            checksums: done.checksums,
                        });
                        if parts.len() > usize::from(MULTIPART_MAX_PARTS) {
                            return Err(malformed());
                        }
                        At::Document
                    }
                    At::Document => {
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
    if !closed || parts.is_empty() {
        return Err(malformed());
    }
    Ok(parts)
}

/// The S3 error for a completion that did not happen.
fn refused(outcome: &Completed) -> Error {
    let (code, message) = match outcome {
        Completed::NoSuchUpload => return no_such_upload(),
        Completed::Refused(Refusal::Order) => (
            ErrorCode::InvalidPartOrder,
            "the list of parts was not in ascending order",
        ),
        Completed::Refused(Refusal::Part(_)) => (
            ErrorCode::InvalidPart,
            "one or more of the specified parts could not be found, or its ETag or checksum did not match",
        ),
        Completed::Refused(Refusal::TooSmall(_)) => (
            ErrorCode::EntityTooSmall,
            "each part must be at least 5 MB in size, except the last part",
        ),
        Completed::Refused(Refusal::Count) => return malformed(),
        Completed::SizeMismatch => (
            ErrorCode::InvalidRequest,
            "x-amz-mp-object-size is not the total size of the parts",
        ),
        Completed::ChecksumMismatch => (
            ErrorCode::BadDigest,
            "the checksum you specified did not match the object's",
        ),
        Completed::OverQuota => (ErrorCode::InvalidRequest, "bucket quota exceeded"),
        Completed::NotWritten(Written::NoSuchBucket) => (
            ErrorCode::NoSuchBucket,
            "the specified bucket does not exist",
        ),
        Completed::NotWritten(Written::PreconditionFailed) => (
            ErrorCode::PreconditionFailed,
            "at least one of the preconditions you specified did not hold",
        ),
        Completed::NotWritten(Written::NoSuchKey) => {
            (ErrorCode::NoSuchKey, "the specified key does not exist")
        }
        Completed::NotWritten(_) | Completed::Done { .. } => (
            ErrorCode::InternalError,
            "we encountered an internal error; please try again",
        ),
    };
    Error::new(code, message)
}

/// `POST /{bucket}/{key}?uploadId=X`.
pub(crate) async fn complete(
    call: &Call<'_>,
    key: &ObjectKey,
    body: Body,
) -> Result<Response<Body>> {
    let id = upload_id(call)?;
    let headers = read_put_headers(call.headers)?;
    let size = call
        .headers
        .get("x-amz-mp-object-size")
        .map(|value| {
            value
                .to_str()
                .ok()
                .and_then(|text| text.trim().parse::<u64>().ok())
                .ok_or_else(|| {
                    Error::new(
                        ErrorCode::InvalidArgument,
                        "x-amz-mp-object-size is not a size",
                    )
                })
        })
        .transpose()?;
    let document = read_verified(body, call.verified, COMPLETE_MULTIPART_BODY_MAX).await?;
    if let Some(md5) = headers.integrity.content_md5 {
        let mut hashes = Hashes::new();
        hashes.update(&document);
        if hashes.finish().md5 != md5 {
            return Err(Error::new(
                ErrorCode::BadDigest,
                "the Content-MD5 you specified did not match the body",
            ));
        }
    }
    let bucket = call.bucket()?;
    // The object check here; the byte check on the listed parts' total, which only the store adds up.
    let room = match Room::of(call, bucket).await? {
        Some(room) => {
            room.admit(call, bucket, Some(key), 0).await?;
            room.bytes_left()
        }
        None => None,
    };
    let request = Completion {
        parts: parse(&document)?,
        condition: headers.condition,
        // A checksum header here is the whole object's, not the body's.
        checksum: headers.integrity.checksum.and_then(|declared| {
            declared
                .header_value
                .map(|value| (declared.algorithm, value))
        }),
        size,
        room,
    };
    let outcome = call
        .state
        .storage()
        .multipart()
        .complete(bucket, key, id, &request)
        .await?;
    let Completed::Done {
        etag,
        checksum: (algorithm, value),
        composite,
    } = outcome
    else {
        return Err(refused(&outcome));
    };
    let key = key_text(key)?;
    let bucket = escape(bucket.as_str());
    let name = algorithm.name();
    let kind = if composite {
        "COMPOSITE"
    } else {
        "FULL_OBJECT"
    };
    Ok(xml_response(
        StatusCode::OK,
        format!(
            "<CompleteMultipartUploadResult xmlns=\"{S3_NAMESPACE}\"><Location>/{bucket}/{key}</Location>\
             <Bucket>{bucket}</Bucket><Key>{key}</Key><ETag>{}</ETag><Checksum{name}>{}</Checksum{name}>\
             <ChecksumType>{kind}</ChecksumType></CompleteMultipartUploadResult>",
            escape(&etag),
            escape(&value)
        ),
    ))
}

#[cfg(test)]
#[path = "complete_tests.rs"]
mod tests;
