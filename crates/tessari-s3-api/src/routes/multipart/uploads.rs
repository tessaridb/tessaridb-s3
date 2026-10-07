//! ListMultipartUploads: one page of a bucket's open uploads, by key and then by initiation, resumed by key-marker and
//! upload-id-marker, rolled up at a delimiter. Keys, prefixes and markers are URL-encoded with `encoding-type=url`;
//! otherwise one XML 1.0 cannot carry is refused rather than written as invalid XML.

use std::fmt::Write as _;

use axum::body::Body;
use axum::http::{Response, StatusCode};
use tessari_s3_constants::LIST_UPLOADS_MAX;
use tessari_s3_core::objects::upload_listing::{UploadEntry, UploadSpec};
use tessari_s3_storage::multipart::UploadsListed;
use tessari_s3_types::{ErrorCode, UploadId};

use crate::pipeline::call::Call;
use crate::pipeline::response::xml_response;
use crate::routes::listing::{listed_name as name, url_encoded};
use crate::xml::{S3_NAMESPACE, escape};
use crate::{Error, Result};

/// `max-uploads`: a non-negative integer, at most 1,000 (a larger request gets 1,000, as max-keys does).
fn max_uploads(call: &Call<'_>) -> Result<usize> {
    match call.query_value("max-uploads") {
        None => Ok(LIST_UPLOADS_MAX),
        Some(text) => text
            .parse::<u64>()
            .map(|max| {
                usize::try_from(max).map_or(LIST_UPLOADS_MAX, |max| max.min(LIST_UPLOADS_MAX))
            })
            .map_err(|_| {
                Error::new(
                    ErrorCode::InvalidArgument,
                    "max-uploads must be a non-negative integer",
                )
            }),
    }
}

/// `GET /{bucket}?uploads`.
pub(crate) async fn list_uploads(call: &Call<'_>) -> Result<Response<Body>> {
    let prefix = call.query_value("prefix").unwrap_or("");
    // An empty delimiter rolls nothing up.
    let delimiter = call.query_value("delimiter").filter(|d| !d.is_empty());
    let max = max_uploads(call)?;
    let url = url_encoded(call)?;
    let key_marker = call.query_value("key-marker").filter(|k| !k.is_empty());
    let id_marker = call.query_value("upload-id-marker").unwrap_or("");
    let spec = UploadSpec {
        prefix: prefix.to_owned(),
        delimiter: delimiter.map(str::to_owned),
        max_uploads: max,
        key_marker: key_marker.map(str::to_owned),
    };
    // An id this server could not have issued names no open upload: it resumes as a vanished one would.
    let upload_marker = UploadId::parse(id_marker).ok();
    let bucket = call.bucket()?;
    let page = match call
        .state
        .storage()
        .multipart()
        .list_uploads(bucket, spec, upload_marker)
        .await?
    {
        UploadsListed::Page(page) => page,
        UploadsListed::NoSuchBucket => {
            return Err(Error::new(
                ErrorCode::NoSuchBucket,
                "the specified bucket does not exist",
            ));
        }
    };
    let mut out = format!(
        "<ListMultipartUploadsResult xmlns=\"{S3_NAMESPACE}\"><Bucket>{}</Bucket><KeyMarker>{}</KeyMarker>\
         <UploadIdMarker>{}</UploadIdMarker>",
        escape(bucket.as_str()),
        name(url, key_marker.unwrap_or(""))?,
        escape(if key_marker.is_some() { id_marker } else { "" })
    );
    match page.entries.last() {
        Some(UploadEntry::Upload { key, item }) => {
            let _ = write!(
                out,
                "<NextKeyMarker>{}</NextKeyMarker><NextUploadIdMarker>{}</NextUploadIdMarker>",
                name(url, key)?,
                item.id.hex()
            );
        }
        Some(UploadEntry::Prefix(prefix)) => {
            let _ = write!(out, "<NextKeyMarker>{}</NextKeyMarker>", name(url, prefix)?);
        }
        None => {}
    }
    if let Some(delimiter) = delimiter {
        let _ = write!(out, "<Delimiter>{}</Delimiter>", name(url, delimiter)?);
    }
    let _ = write!(out, "<Prefix>{}</Prefix>", name(url, prefix)?);
    if url {
        out.push_str("<EncodingType>url</EncodingType>");
    }
    let _ = write!(
        out,
        "<MaxUploads>{max}</MaxUploads><IsTruncated>{}</IsTruncated>",
        page.truncated
    );
    let owner = escape(call.verified.access_key());
    let mut prefixes = String::new();
    for entry in &page.entries {
        match entry {
            UploadEntry::Upload { key, item } => {
                let _ = write!(
                    out,
                    "<Upload><Key>{}</Key><UploadId>{}</UploadId>\
                     <Initiator><ID>{owner}</ID><DisplayName>{owner}</DisplayName></Initiator>\
                     <Owner><ID>{owner}</ID><DisplayName>{owner}</DisplayName></Owner>\
                     <StorageClass>STANDARD</StorageClass><Initiated>{}</Initiated></Upload>",
                    name(url, key)?,
                    item.id.hex(),
                    item.initiated.iso8601_millis()
                );
            }
            UploadEntry::Prefix(prefix) => {
                let _ = write!(
                    prefixes,
                    "<CommonPrefixes><Prefix>{}</Prefix></CommonPrefixes>",
                    name(url, prefix)?
                );
            }
        }
    }
    out.push_str(&prefixes);
    out.push_str("</ListMultipartUploadsResult>");
    Ok(xml_response(StatusCode::OK, out))
}
