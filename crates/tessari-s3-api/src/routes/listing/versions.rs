//! ListObjectVersions: one page of a bucket's versions and delete markers — keys in UTF-8 byte order, each key's
//! versions newest first — resumed by key-marker and version-id-marker, rolled up at a delimiter (S3 ref 06 M5).

use std::fmt::Write as _;

use axum::body::Body;
use axum::http::{Response, StatusCode};
use tessari_s3_core::objects::upload_listing::{UploadEntry, UploadSpec};
use tessari_s3_storage::objects::{VersionKind, VersionsListed};
use tessari_s3_types::ErrorCode;

use super::{invalid, listed_name as name, max_keys, url_encoded};
use crate::pipeline::call::Call;
use crate::pipeline::response::xml_response;
use crate::xml::{S3_NAMESPACE, escape};
use crate::{Error, Result};

/// `GET /{bucket}?versions`.
pub(crate) async fn list_versions(call: &Call<'_>) -> Result<Response<Body>> {
    let prefix = call.query_value("prefix").unwrap_or("");
    // An empty delimiter rolls nothing up.
    let delimiter = call.query_value("delimiter").filter(|d| !d.is_empty());
    let max = max_keys(call)?;
    let url = url_encoded(call)?;
    let key_marker = call.query_value("key-marker").filter(|k| !k.is_empty());
    let id_marker = call
        .query_value("version-id-marker")
        .filter(|id| !id.is_empty());
    if id_marker.is_some() && key_marker.is_none() {
        return Err(invalid(
            "A version-id marker cannot be specified without a key marker.",
        ));
    }
    let spec = UploadSpec {
        prefix: prefix.to_owned(),
        delimiter: delimiter.map(str::to_owned),
        max_uploads: max,
        key_marker: key_marker.map(str::to_owned),
    };
    let bucket = call.bucket()?;
    let page = match call
        .state
        .storage()
        .objects()
        .list_versions(bucket, spec, id_marker)
        .await?
    {
        VersionsListed::Page(page) => page,
        VersionsListed::MalformedMarker => return Err(invalid("Invalid version id specified")),
        VersionsListed::NoSuchBucket => {
            return Err(Error::new(
                ErrorCode::NoSuchBucket,
                "the specified bucket does not exist",
            ));
        }
    };
    let mut out = format!(
        "<ListVersionsResult xmlns=\"{S3_NAMESPACE}\"><Name>{}</Name><Prefix>{}</Prefix>\
         <KeyMarker>{}</KeyMarker><VersionIdMarker>{}</VersionIdMarker>",
        escape(bucket.as_str()),
        name(url, prefix)?,
        name(url, key_marker.unwrap_or(""))?,
        escape(id_marker.unwrap_or(""))
    );
    if page.truncated {
        match page.entries.last() {
            Some(UploadEntry::Upload { key, item }) => {
                let _ = write!(
                    out,
                    "<NextKeyMarker>{}</NextKeyMarker><NextVersionIdMarker>{}</NextVersionIdMarker>",
                    name(url, key)?,
                    escape(&item.version_id)
                );
            }
            Some(UploadEntry::Prefix(prefix)) => {
                let _ = write!(out, "<NextKeyMarker>{}</NextKeyMarker>", name(url, prefix)?);
            }
            None => {}
        }
    }
    let _ = write!(out, "<MaxKeys>{max}</MaxKeys>");
    if let Some(delimiter) = delimiter {
        let _ = write!(out, "<Delimiter>{}</Delimiter>", name(url, delimiter)?);
    }
    let _ = write!(out, "<IsTruncated>{}</IsTruncated>", page.truncated);
    let owner = escape(call.verified.access_key());
    let mut prefixes = String::new();
    for entry in &page.entries {
        match entry {
            UploadEntry::Upload { key, item } => {
                let common = format!(
                    "<Key>{}</Key><VersionId>{}</VersionId><IsLatest>{}</IsLatest>\
                     <LastModified>{}</LastModified>",
                    name(url, key)?,
                    escape(&item.version_id),
                    item.latest,
                    item.modified.iso8601_millis()
                );
                let owner =
                    format!("<Owner><ID>{owner}</ID><DisplayName>{owner}</DisplayName></Owner>");
                match &item.kind {
                    VersionKind::Object { size, etag } => {
                        let _ = write!(
                            out,
                            "<Version>{common}<ETag>{}</ETag><Size>{size}</Size>{owner}\
                             <StorageClass>STANDARD</StorageClass></Version>",
                            escape(etag)
                        );
                    }
                    VersionKind::Marker => {
                        let _ = write!(out, "<DeleteMarker>{common}{owner}</DeleteMarker>");
                    }
                    // A client's marker is a place, never an entry.
                    VersionKind::Place => {}
                }
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
    if url {
        out.push_str("<EncodingType>url</EncodingType>");
    }
    out.push_str("</ListVersionsResult>");
    Ok(xml_response(StatusCode::OK, out))
}
