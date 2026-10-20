//! GetObjectAttributes: `GET /{bucket}/{key}?attributes` answers what HeadObject and ListParts would, without the
//! bytes — only the attributes `x-amz-object-attributes` names (ETag, Checksum, ObjectParts, StorageClass,
//! ObjectSize), of the current version or the one `versionId` names. A customer-sealed object needs its key, as a read
//! does. The ETag is answered without its quotes; a multipart object's parts are paged by `x-amz-max-parts` and
//! `x-amz-part-number-marker`.

use std::fmt::Write as _;

use axum::body::Body;
use axum::http::{HeaderValue, Response, StatusCode, header};
use tessari_s3_constants::LIST_PARTS_MAX;
use tessari_s3_storage::objects::{Content, StoredObject};
use tessari_s3_types::{ErrorCode, ObjectKey};

use super::addressed;
use super::customer::{admit_read, read_customer};
use crate::pipeline::call::Call;
use crate::pipeline::response::{with_version, xml_response};
use crate::xml::{S3_NAMESPACE, escape};
use crate::{Error, Result};

/// The attributes a request asks for.
#[derive(Default)]
struct Wanted {
    etag: bool,
    checksum: bool,
    parts: bool,
    storage_class: bool,
    size: bool,
}

fn invalid(message: &str) -> Error {
    Error::new(ErrorCode::InvalidArgument, message.to_owned())
}

fn wanted(call: &Call<'_>) -> Result<Wanted> {
    let listed = call
        .headers
        .get_all("x-amz-object-attributes")
        .iter()
        .map(|value| {
            value
                .to_str()
                .map_err(|_| invalid("Invalid attribute name specified."))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut wanted = Wanted::default();
    for name in listed
        .iter()
        .flat_map(|value| value.split(','))
        .map(str::trim)
    {
        match name {
            "ETag" => wanted.etag = true,
            "Checksum" => wanted.checksum = true,
            "ObjectParts" => wanted.parts = true,
            "StorageClass" => wanted.storage_class = true,
            "ObjectSize" => wanted.size = true,
            _ => return Err(invalid("Invalid attribute name specified.")),
        }
    }
    Ok(wanted)
}

/// A paging header's number, `default` when absent, refused above `max`.
fn paging(call: &Call<'_>, name: &str, default: usize, max: usize) -> Result<usize> {
    match call.headers.get(name) {
        None => Ok(default),
        Some(value) => value
            .to_str()
            .ok()
            .and_then(|text| text.trim().parse::<usize>().ok())
            .filter(|number| *number <= max)
            .ok_or_else(|| invalid(&format!("{name} must be an integer from 0 to {max}"))),
    }
}

fn checksum(object: &StoredObject) -> String {
    let mut out = String::new();
    // A composite value names its part count after the digest (`…-N`); a whole-object one does not.
    let mut kind = "FULL_OBJECT";
    for (algorithm, value) in &object.checksums {
        if value.contains('-') {
            kind = "COMPOSITE";
        }
        let _ = write!(
            out,
            "<Checksum{0}>{1}</Checksum{0}>",
            escape(algorithm),
            escape(value)
        );
    }
    if out.is_empty() {
        return out;
    }
    format!("<Checksum>{out}<ChecksumType>{kind}</ChecksumType></Checksum>")
}

fn parts(call: &Call<'_>, object: &StoredObject) -> Result<String> {
    let Content::Parts(multipart) = &object.content else {
        return Ok(String::new());
    };
    let max = paging(call, "x-amz-max-parts", LIST_PARTS_MAX, LIST_PARTS_MAX)?;
    let marker = paging(call, "x-amz-part-number-marker", 0, 10_000)?;
    let after: Vec<_> = multipart
        .parts
        .iter()
        .filter(|part| usize::from(part.number) > marker)
        .collect();
    let truncated = after.len() > max;
    let page = &after[..after.len().min(max)];
    let mut out = format!(
        "<ObjectParts><IsTruncated>{truncated}</IsTruncated><MaxParts>{max}</MaxParts>\
         <PartNumberMarker>{marker}</PartNumberMarker>"
    );
    if let Some(last) = page.last() {
        let _ = write!(
            out,
            "<NextPartNumberMarker>{}</NextPartNumberMarker>",
            last.number
        );
    }
    for part in page {
        let _ = write!(out, "<Part>");
        // The part's own value of each algorithm the object carries.
        for algorithm in object.checksums.keys() {
            if let Some(value) = part.checksums.get(algorithm) {
                let _ = write!(
                    out,
                    "<Checksum{0}>{1}</Checksum{0}>",
                    escape(algorithm),
                    escape(value)
                );
            }
        }
        let _ = write!(
            out,
            "<PartNumber>{}</PartNumber><Size>{}</Size></Part>",
            part.number, part.size
        );
    }
    let _ = write!(
        out,
        "<PartsCount>{}</PartsCount></ObjectParts>",
        multipart.parts.len()
    );
    Ok(out)
}

/// `GET /{bucket}/{key}?attributes`.
pub(crate) async fn attributes(call: &Call<'_>, key: &ObjectKey) -> Result<Response<Body>> {
    let wanted = wanted(call)?;
    let customer = read_customer(call.headers)?;
    let object = addressed(call, key).await?;
    admit_read(call, &object, customer.as_ref())?;
    let mut out = format!("<GetObjectAttributesResponse xmlns=\"{S3_NAMESPACE}\">");
    if wanted.etag {
        let _ = write!(
            out,
            "<ETag>{}</ETag>",
            escape(object.etag.trim_matches('"'))
        );
    }
    if wanted.checksum {
        out.push_str(&checksum(&object));
    }
    if wanted.parts {
        out.push_str(&parts(call, &object)?);
    }
    if wanted.storage_class {
        out.push_str("<StorageClass>STANDARD</StorageClass>");
    }
    if wanted.size {
        let _ = write!(out, "<ObjectSize>{}</ObjectSize>", object.size);
    }
    out.push_str("</GetObjectAttributesResponse>");
    let mut response = with_version(xml_response(StatusCode::OK, out), object.version.as_deref());
    if let Ok(modified) = HeaderValue::from_str(&object.modified.http_date()) {
        response
            .headers_mut()
            .insert(header::LAST_MODIFIED, modified);
    }
    Ok(response)
}
