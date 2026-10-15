//! CopyObject's request: the `x-amz-copy-source` header and the metadata and tagging directives.

use crate::auth::percent_decode;

/// The object a copy reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopySource {
    /// The source bucket's name, not yet validated as one.
    pub bucket: String,
    /// The source key, not yet validated as one.
    pub key: String,
}

/// Why a copy source was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopySourceError {
    /// It names a version, and this server keeps none.
    Version,
    /// It is not a percent-encoded `bucket/key`.
    Malformed,
}

impl CopySource {
    /// Reads `x-amz-copy-source`: `[/]bucket/key[?versionId=…]`, percent-encoded as a whole (a key's own `?` arrives as
    /// `%3F`, so the first raw `?` starts the version), decoded exactly once.
    ///
    /// # Errors
    /// [`CopySourceError::Version`] for a `versionId`; [`CopySourceError::Malformed`] for anything that is not a
    /// bucket and a non-empty key.
    pub fn parse(header: &str) -> Result<Self, CopySourceError> {
        let (path, query) = match header.split_once('?') {
            Some((path, query)) => (path, Some(query)),
            None => (header, None),
        };
        match query {
            None => {}
            Some(query) if query.starts_with("versionId=") => return Err(CopySourceError::Version),
            Some(_) => return Err(CopySourceError::Malformed),
        }
        let decoded = percent_decode(path).map_err(|_| CopySourceError::Malformed)?;
        let decoded = String::from_utf8(decoded).map_err(|_| CopySourceError::Malformed)?;
        let decoded = decoded.strip_prefix('/').unwrap_or(&decoded);
        match decoded.split_once('/') {
            Some((bucket, key)) if !bucket.is_empty() && !key.is_empty() => Ok(Self {
                bucket: bucket.to_owned(),
                key: key.to_owned(),
            }),
            _ => Err(CopySourceError::Malformed),
        }
    }
}

/// Why an `x-amz-copy-source-range` was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyRangeError {
    /// It is not `bytes=first-last` with `first <= last`.
    Malformed,
    /// It reaches past the source.
    OutOfRange,
}

/// Reads `x-amz-copy-source-range` against a source of `size` bytes: exactly `bytes=first-last`, zero-based and
/// inclusive — unlike a GET's Range there is no suffix, no open end and no list, so anything else is refused rather
/// than answered with the whole object.
///
/// # Errors
/// [`CopyRangeError::Malformed`] for any other form or `first > last`; [`CopyRangeError::OutOfRange`] when `last`
/// is not a byte of the source.
pub fn copy_range(header: &str, size: u64) -> Result<(u64, u64), CopyRangeError> {
    let spec = header
        .trim()
        .strip_prefix("bytes=")
        .ok_or(CopyRangeError::Malformed)?;
    let (first, last) = spec.split_once('-').ok_or(CopyRangeError::Malformed)?;
    let number = |text: &str| {
        (!text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()))
            .then(|| text.parse::<u64>().ok())
            .flatten()
            .ok_or(CopyRangeError::Malformed)
    };
    let (first, last) = (number(first)?, number(last)?);
    if first > last {
        return Err(CopyRangeError::Malformed);
    }
    if last >= size {
        return Err(CopyRangeError::OutOfRange);
    }
    Ok((first, last))
}

/// A metadata or tagging directive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Directive {
    /// Keep the source's (the default).
    Copy,
    /// Take the request's.
    Replace,
}

impl Directive {
    /// Reads a directive header: absent or `COPY` copies, `REPLACE` replaces, anything else is `None` (refused).
    #[must_use]
    pub fn parse(value: Option<&str>) -> Option<Self> {
        match value {
            None | Some("COPY") => Some(Self::Copy),
            Some("REPLACE") => Some(Self::Replace),
            Some(_) => None,
        }
    }
}

#[cfg(test)]
#[path = "copy_tests.rs"]
mod tests;
