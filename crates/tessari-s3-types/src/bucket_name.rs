//! [`BucketName`]: a name that satisfies the AWS general purpose bucket naming rules.

use tessari_s3_constants::{
    BUCKET_NAME_MAX_LEN, BUCKET_NAME_MIN_LEN, BUCKET_NAME_RESERVED_PREFIXES,
    BUCKET_NAME_RESERVED_SUFFIXES,
};

use crate::{Error, Result};

/// A bucket name that satisfies the AWS general purpose bucket naming rules, so an invalid name cannot travel past
/// the parser.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BucketName(String);

impl BucketName {
    /// Checks `name` against the naming rules.
    ///
    /// # Errors
    /// [`Error::InvalidBucketName`] naming the first rule the name breaks.
    pub fn new(name: &str) -> Result<Self> {
        let refuse = |reason| Err(Error::InvalidBucketName { reason });
        if !(BUCKET_NAME_MIN_LEN..=BUCKET_NAME_MAX_LEN).contains(&name.len()) {
            return refuse("must be 3 to 63 characters long");
        }
        let allowed =
            |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-';
        if !name.bytes().all(allowed) {
            return refuse("may contain only lowercase letters, digits, periods and hyphens");
        }
        let alphanumeric =
            |b: Option<u8>| b.is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
        if !alphanumeric(name.bytes().next()) || !alphanumeric(name.bytes().next_back()) {
            return refuse("must begin and end with a letter or a digit");
        }
        if name.contains("..") {
            return refuse("must not contain two adjacent periods");
        }
        if is_dotted_quad(name) {
            return refuse("must not be formatted as an IP address");
        }
        if BUCKET_NAME_RESERVED_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix))
        {
            return refuse("must not start with a reserved prefix");
        }
        if BUCKET_NAME_RESERVED_SUFFIXES
            .iter()
            .any(|suffix| name.ends_with(suffix))
        {
            return refuse("must not end with a reserved suffix");
        }
        Ok(Self(name.to_owned()))
    }

    /// The name as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Whether `name` has the shape of an IPv4 address: four dot-separated groups of one to three digits.
fn is_dotted_quad(name: &str) -> bool {
    let groups: Vec<&str> = name.split('.').collect();
    groups.len() == 4
        && groups.iter().all(|group| {
            (1..=3).contains(&group.len()) && group.bytes().all(|b| b.is_ascii_digit())
        })
}

#[cfg(test)]
#[path = "bucket_name_tests.rs"]
mod tests;
