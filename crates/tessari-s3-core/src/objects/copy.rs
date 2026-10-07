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
