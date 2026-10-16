//! Tags on an object version or a bucket (S3 ref 05 M8): an ordered set of unique keys with values, within the limits
//! S3 publishes. A set exists only as one that passed them.

use crate::auth::percent_decode;

/// The longest key, in characters.
const KEY_CHARS: usize = 128;
/// The longest value, in characters.
const VALUE_CHARS: usize = 256;

/// The most tags an object version holds.
pub const OBJECT_TAGS: usize = 10;
/// The most tags a bucket holds.
pub const BUCKET_TAGS: usize = 50;

/// One tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tag {
    /// Its key.
    pub key: String,
    /// Its value, possibly empty.
    pub value: String,
}

/// Tags that passed S3's limits.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TagSet(Vec<Tag>);

/// Why a tag set is refused — every case is S3's `InvalidTag`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TagError {
    /// More tags than the owner holds.
    #[error("Object tags cannot be greater than 10, bucket tags than 50")]
    TooMany,
    /// A key empty or longer than 128 characters.
    #[error("The TagKey you have provided is invalid")]
    KeyLength,
    /// A value longer than 256 characters.
    #[error("The TagValue you have provided is invalid")]
    ValueLength,
    /// A character outside letters, digits, spaces and `+ - = . _ : / @`.
    #[error("The TagKey or TagValue you have provided contains a character that is not allowed")]
    Character,
    /// A key in the `aws:` namespace.
    #[error("Your TagKey cannot be prefixed with aws:")]
    Reserved,
    /// Two tags with one key.
    #[error("Cannot provide multiple Tags with the same key")]
    DuplicateKey,
    /// A header that is not percent-encoded correctly.
    #[error(
        "The header 'x-amz-tagging' shall be encoded as UTF-8 then URLEncoded URL query parameters"
    )]
    Encoding,
}

impl TagSet {
    /// The set of `pairs`, at most `max` of them.
    ///
    /// # Errors
    /// [`TagError`] naming the first limit a tag breaks.
    pub fn new(pairs: Vec<(String, String)>, max: usize) -> Result<Self, TagError> {
        if pairs.len() > max {
            return Err(TagError::TooMany);
        }
        let mut tags: Vec<Tag> = Vec::with_capacity(pairs.len());
        for (key, value) in pairs {
            let length = key.chars().count();
            if length == 0 || length > KEY_CHARS {
                return Err(TagError::KeyLength);
            }
            if value.chars().count() > VALUE_CHARS {
                return Err(TagError::ValueLength);
            }
            if !key.chars().chain(value.chars()).all(allowed) {
                return Err(TagError::Character);
            }
            if key.starts_with("aws:") {
                return Err(TagError::Reserved);
            }
            if tags.iter().any(|tag| tag.key == key) {
                return Err(TagError::DuplicateKey);
            }
            tags.push(Tag { key, value });
        }
        Ok(Self(tags))
    }

    /// The set an `x-amz-tagging` header names: `key=value` pairs joined by `&`, percent-encoded.
    ///
    /// # Errors
    /// [`TagError::Encoding`] for a malformed escape or text that is not UTF-8; otherwise as [`Self::new`].
    pub fn from_header(header: &str, max: usize) -> Result<Self, TagError> {
        let decoded = |text: &str| {
            percent_decode(text)
                .ok()
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .ok_or(TagError::Encoding)
        };
        let mut pairs = Vec::new();
        for pair in header.split('&').filter(|pair| !pair.is_empty()) {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            pairs.push((decoded(key)?, decoded(value)?));
        }
        Self::new(pairs, max)
    }

    /// The tags, in the order given.
    #[must_use]
    pub fn tags(&self) -> &[Tag] {
        &self.0
    }
}

/// Letters, digits and spaces in any script, and `+ - = . _ : / @`.
fn allowed(character: char) -> bool {
    character.is_alphanumeric() || character.is_whitespace() || "+-=._:/@".contains(character)
}

#[cfg(test)]
#[path = "tagging_tests.rs"]
mod tests;
