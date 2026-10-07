//! The `ListBucketResult` document for either version. Every key and prefix is written either URL-encoded
//! (`encoding-type=url`) or as XML text; a key XML 1.0 cannot carry is refused rather than written as invalid XML.

use std::fmt::Write as _;

use tessari_s3_core::objects::listing::{Entry, Page};
use tessari_s3_storage::objects::ObjectSummary;
use tessari_s3_types::ErrorCode;

use crate::xml::{S3_NAMESPACE, escape, xml_text};
use crate::{Error, Result};

/// A page and the request it answers.
pub(super) struct Listing<'a> {
    pub(super) bucket: String,
    pub(super) prefix: &'a str,
    pub(super) delimiter: Option<&'a str>,
    pub(super) max_keys: usize,
    pub(super) url: bool,
    pub(super) page: Page<ObjectSummary>,
}

/// What differs between the two versions.
pub(super) enum Version<'a> {
    /// ListObjectsV2.
    Two {
        token: Option<&'a str>,
        next_token: Option<&'a str>,
        start_after: Option<&'a str>,
        fetch_owner: bool,
    },
    /// ListObjects.
    One { marker: Option<&'a str> },
}

/// `text` as URL encoding writes it: unreserved bytes and `/` as they are, a space as `+`, the rest `%XX`.
fn url_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(char::from(byte));
            }
            b' ' => out.push('+'),
            other => {
                let _ = write!(out, "%{other:02X}");
            }
        }
    }
    out
}

/// A key, prefix or marker as this listing writes it.
fn name(listing: &Listing<'_>, text: &str) -> Result<String> {
    if listing.url {
        return Ok(url_encode(text));
    }
    xml_text(text).ok_or_else(|| {
        Error::new(
            ErrorCode::InvalidArgument,
            "a key in this listing cannot be carried in XML; list with encoding-type=url",
        )
    })
}

fn owner(owner: &str) -> String {
    let owner = escape(owner);
    format!("<Owner><ID>{owner}</ID><DisplayName>{owner}</DisplayName></Owner>")
}

fn entries(listing: &Listing<'_>, with_owner: Option<&str>, out: &mut String) -> Result<()> {
    let mut prefixes = String::new();
    for entry in &listing.page.entries {
        match entry {
            Entry::Object { key, summary } => {
                let _ = write!(
                    out,
                    "<Contents><Key>{}</Key><LastModified>{}</LastModified><ETag>{}</ETag>",
                    name(listing, key)?,
                    summary.modified.iso8601_millis(),
                    escape(&summary.etag)
                );
                for algorithm in &summary.checksum_algorithms {
                    let _ = write!(
                        out,
                        "<ChecksumAlgorithm>{}</ChecksumAlgorithm>",
                        escape(algorithm)
                    );
                }
                if !summary.checksum_algorithms.is_empty() {
                    out.push_str("<ChecksumType>FULL_OBJECT</ChecksumType>");
                }
                let _ = write!(
                    out,
                    "<Size>{}</Size><StorageClass>STANDARD</StorageClass>",
                    summary.size
                );
                if let Some(id) = with_owner {
                    out.push_str(&owner(id));
                }
                out.push_str("</Contents>");
            }
            Entry::Prefix(prefix) => {
                let _ = write!(
                    prefixes,
                    "<CommonPrefixes><Prefix>{}</Prefix></CommonPrefixes>",
                    name(listing, prefix)?
                );
            }
        }
    }
    out.push_str(&prefixes);
    Ok(())
}

/// The response document.
pub(super) fn render(
    listing: &Listing<'_>,
    version: &Version<'_>,
    owner_id: &str,
) -> Result<String> {
    let mut out = format!(
        "<ListBucketResult xmlns=\"{S3_NAMESPACE}\"><Name>{}</Name><Prefix>{}</Prefix>",
        escape(&listing.bucket),
        name(listing, listing.prefix)?
    );
    let truncated = listing.page.truncated;
    let with_owner = match version {
        Version::Two {
            token,
            next_token,
            start_after,
            fetch_owner,
        } => {
            if let Some(delimiter) = listing.delimiter {
                let _ = write!(out, "<Delimiter>{}</Delimiter>", name(listing, delimiter)?);
            }
            let _ = write!(
                out,
                "<MaxKeys>{}</MaxKeys><KeyCount>{}</KeyCount><IsTruncated>{truncated}</IsTruncated>",
                listing.max_keys,
                listing.page.entries.len()
            );
            if listing.url {
                out.push_str("<EncodingType>url</EncodingType>");
            }
            if let Some(token) = token {
                let _ = write!(
                    out,
                    "<ContinuationToken>{}</ContinuationToken>",
                    escape(token)
                );
            }
            if let Some(next) = next_token {
                let _ = write!(out, "<NextContinuationToken>{next}</NextContinuationToken>");
            }
            if let Some(start_after) = start_after {
                let _ = write!(
                    out,
                    "<StartAfter>{}</StartAfter>",
                    name(listing, start_after)?
                );
            }
            fetch_owner.then_some(owner_id)
        }
        Version::One { marker } => {
            let _ = write!(
                out,
                "<Marker>{}</Marker>",
                name(listing, marker.unwrap_or(""))?
            );
            // S3 names a NextMarker only when a delimiter is given; otherwise the client resumes from the last key.
            if let Some(next) = listing.delimiter.and(listing.page.resume()) {
                let position = match &next {
                    tessari_s3_core::objects::listing::Resume::Key(key) => key,
                    tessari_s3_core::objects::listing::Resume::Prefix(prefix) => prefix,
                };
                let _ = write!(out, "<NextMarker>{}</NextMarker>", name(listing, position)?);
            }
            let _ = write!(out, "<MaxKeys>{}</MaxKeys>", listing.max_keys);
            if let Some(delimiter) = listing.delimiter {
                let _ = write!(out, "<Delimiter>{}</Delimiter>", name(listing, delimiter)?);
            }
            let _ = write!(out, "<IsTruncated>{truncated}</IsTruncated>");
            if listing.url {
                out.push_str("<EncodingType>url</EncodingType>");
            }
            Some(owner_id)
        }
    };
    entries(listing, with_owner, &mut out)?;
    out.push_str("</ListBucketResult>");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::url_encode;

    #[test]
    fn url_encoding_keeps_unreserved_and_slash_and_encodes_the_rest() {
        assert_eq!(url_encode("a b/ü+&~._-"), "a+b/%C3%BC%2B%26~._-");
        assert_eq!(url_encode("x\u{1}y"), "x%01y");
    }
}
