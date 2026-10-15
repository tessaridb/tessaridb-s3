//! GetObject and HeadObject: conditional reads with S3's precedence, a single Range, the stored headers, the
//! `response-*` overrides, and the stored checksum when the client asks for it.

use axum::body::Body;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Response, StatusCode, header};
use tessari_s3_core::objects::checksum::{ChecksumAlgorithm, Declared};
use tessari_s3_core::objects::conditions::{ReadConditions, ReadVerdict, evaluate};
use tessari_s3_core::objects::range::{RangeRequest, part_bytes, resolve};
use tessari_s3_storage::objects::{Content, StoredObject, VersionRead};
use tessari_s3_types::{ErrorCode, ObjectKey, PartNumber};

use super::body;
use crate::pipeline::call::Call;
use crate::{Error, Result};

/// The `response-*` query overrides GetObject models, and the header each one sets.
const OVERRIDES: [(&str, &str); 6] = [
    ("response-cache-control", "cache-control"),
    ("response-content-disposition", "content-disposition"),
    ("response-content-encoding", "content-encoding"),
    ("response-content-language", "content-language"),
    ("response-content-type", "content-type"),
    ("response-expires", "expires"),
];

fn set(headers: &mut HeaderMap, name: &str, value: &str) {
    if let (Ok(name), Ok(value)) = (
        HeaderName::from_bytes(name.as_bytes()),
        HeaderValue::from_str(value),
    ) {
        headers.insert(name, value);
    }
}

/// `GET` or `HEAD /{bucket}/{key}`; `head` answers the headers and no body.
pub(crate) async fn read(call: &Call<'_>, key: &ObjectKey, head: bool) -> Result<Response<Body>> {
    let part_number = call
        .query
        .iter()
        .find(|(name, _)| name == "partNumber")
        .map(|(_, value)| {
            PartNumber::parse(value.as_deref().unwrap_or("")).map_err(|_| {
                Error::new(
                    ErrorCode::InvalidArgument,
                    "partNumber must be an integer from 1 to 10000",
                )
            })
        })
        .transpose()?;
    if part_number.is_some() && call.headers.contains_key(header::RANGE) {
        return Err(Error::new(
            ErrorCode::InvalidRequest,
            "Range and partNumber cannot both be specified",
        ));
    }
    let object = match call.query_value("versionId") {
        Some(version) => stored_version(call, key, version).await?,
        None => stored(call, key).await?,
    };
    let text = |name: &str| call.headers.get(name).and_then(|v| v.to_str().ok());
    let conditions = ReadConditions {
        if_match: text("if-match"),
        if_none_match: text("if-none-match"),
        if_modified_since: text("if-modified-since"),
        if_unmodified_since: text("if-unmodified-since"),
    };
    match evaluate(&conditions, &object.etag, object.modified) {
        ReadVerdict::PreconditionFailed => {
            return Err(Error::new(
                ErrorCode::PreconditionFailed,
                "at least one of the preconditions you specified did not hold",
            ));
        }
        ReadVerdict::NotModified => {
            let mut response = Response::new(Body::empty());
            *response.status_mut() = StatusCode::NOT_MODIFIED;
            set(response.headers_mut(), "etag", &object.etag);
            set(
                response.headers_mut(),
                "last-modified",
                &object.modified.http_date(),
            );
            return Ok(response);
        }
        ReadVerdict::Proceed => {}
    }
    let sizes: Vec<u64> = match &object.content {
        Content::Parts(multipart) => multipart.parts.iter().map(|part| part.size).collect(),
        Content::Data(_) => vec![object.size],
    };
    let range = match part_number {
        None => resolve(text("range"), object.size),
        // A single-part object is its own part 1.
        Some(number) if sizes.len() == 1 && number.get() == 1 => RangeRequest::Whole,
        Some(number) => match part_bytes(&sizes, usize::from(number.get())) {
            Some((start, end)) => RangeRequest::Part { start, end },
            None => {
                return Err(Error::new(
                    ErrorCode::InvalidPartNumber,
                    "The requested partnumber is not satisfiable",
                ));
            }
        },
    };
    let (status, slice) = match range {
        RangeRequest::Whole => (StatusCode::OK, None),
        RangeRequest::Part { start, end } => (StatusCode::PARTIAL_CONTENT, Some((start, end))),
        RangeRequest::Unsatisfiable => {
            return Err(Error::new(
                ErrorCode::InvalidRange,
                "the requested range is not satisfiable",
            ));
        }
    };
    let length = match slice {
        None => object.size,
        Some((start, end)) => end
            .checked_sub(start)
            .and_then(|span| span.checked_add(1))
            .ok_or_else(|| Error::new(ErrorCode::InvalidRange, "range"))?,
    };
    let body = if head {
        Body::empty()
    } else {
        match &object.content {
            // An empty object has no byte to read back.
            _ if object.size == 0 => Body::empty(),
            Content::Data(id) => {
                let (start, end) = slice.unwrap_or((0, object.size.saturating_sub(1)));
                body::send(call.state.storage().objects(), *id, object.size, start, end).await?
            }
            Content::Parts(multipart) => {
                let (start, end) = slice.unwrap_or((0, object.size.saturating_sub(1)));
                let parts: Vec<([u8; 16], u64)> = multipart
                    .parts
                    .iter()
                    .map(|part| (part.data, part.size))
                    .collect();
                body::send_parts(call.state.storage().objects(), &parts, start, end).await?
            }
        }
    };
    let mut response = Response::new(body);
    *response.status_mut() = status;
    let headers = response.headers_mut();
    describe(headers, &object, call, slice.is_none());
    set(
        headers,
        header::CONTENT_LENGTH.as_str(),
        &length.to_string(),
    );
    if let Some((start, end)) = slice {
        set(
            headers,
            "content-range",
            &format!("bytes {start}-{end}/{}", object.size),
        );
    }
    if part_number.is_some() && sizes.len() > 1 {
        set(headers, "x-amz-mp-parts-count", &sizes.len().to_string());
    }
    Ok(response)
}

fn no_such_bucket() -> Error {
    Error::new(
        ErrorCode::NoSuchBucket,
        "the specified bucket does not exist",
    )
}

/// The key's current object; a key whose newest version is a delete marker answers 404 naming it (S3 ref 08 M4).
async fn stored(call: &Call<'_>, key: &ObjectKey) -> Result<StoredObject> {
    let objects = call.state.storage().objects();
    match objects.get(call.bucket()?, key).await? {
        Err(()) => Err(no_such_bucket()),
        Ok(Some(object)) => Ok(object),
        Ok(None) => {
            let missing = Error::new(ErrorCode::NoSuchKey, "the specified key does not exist");
            Err(match objects.current_marker(call.bucket()?, key).await? {
                Some(marker) => missing
                    .with_header("x-amz-delete-marker", "true")
                    .with_header("x-amz-version-id", marker),
                None => missing,
            })
        }
    }
}

/// The version `version` of the key; a delete marker cannot be read and answers 405 (S3 ref 08 M4).
async fn stored_version(call: &Call<'_>, key: &ObjectKey, version: &str) -> Result<StoredObject> {
    match call
        .state
        .storage()
        .objects()
        .get_version(call.bucket()?, key, version)
        .await?
    {
        VersionRead::Object(object) => Ok(object),
        VersionRead::Marker { modified } => Err(Error::new(
            ErrorCode::MethodNotAllowed,
            "The specified method is not allowed against this resource.",
        )
        .with_header("x-amz-delete-marker", "true")
        .with_header("last-modified", modified.http_date())
        .with_header("x-amz-version-id", version)),
        VersionRead::Absent => Err(Error::new(
            ErrorCode::NoSuchVersion,
            "The specified version does not exist.",
        )),
        VersionRead::Malformed => Err(Error::new(
            ErrorCode::InvalidArgument,
            "Invalid version id specified",
        )),
        VersionRead::NoSuchBucket => Err(no_such_bucket()),
    }
}

/// The headers that describe the stored object.
fn describe(headers: &mut HeaderMap, object: &StoredObject, call: &Call<'_>, whole: bool) {
    set(headers, "content-type", "binary/octet-stream");
    for (name, value) in &object.headers {
        set(headers, name, value);
    }
    for (name, value) in &object.metadata {
        set(headers, &format!("x-amz-meta-{name}"), value);
    }
    for (query, header) in OVERRIDES {
        if let Some(value) = call.query_value(query) {
            set(headers, header, value);
        }
    }
    set(headers, "etag", &object.etag);
    set(headers, "last-modified", &object.modified.http_date());
    set(headers, "accept-ranges", "bytes");
    if let Some(version) = &object.version {
        set(headers, "x-amz-version-id", version);
    }
    if let Some(status) = &object.replication {
        set(headers, "x-amz-replication-status", status);
    }
    let checksum_mode = call
        .headers
        .get("x-amz-checksum-mode")
        .and_then(|v| v.to_str().ok());
    if whole && checksum_mode.is_some_and(|mode| mode.eq_ignore_ascii_case("ENABLED")) {
        for (name, value) in &object.checksums {
            if let Some(Declared::Supported(algorithm)) = ChecksumAlgorithm::parse(name) {
                set(headers, algorithm.header(), value);
                // A composite value carries its part count after `-`, which base64 never contains.
                let kind = if value.contains('-') {
                    "COMPOSITE"
                } else {
                    "FULL_OBJECT"
                };
                set(headers, "x-amz-checksum-type", kind);
            }
        }
    }
}
