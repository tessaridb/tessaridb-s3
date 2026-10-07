//! [`ObjectKey`]: an object key within the S3 length limit.

use tessari_s3_constants::OBJECT_KEY_MAX_LEN;

use crate::{Error, Result};

/// An object key: opaque UTF-8, 1 to 1,024 bytes, kept byte for byte as the client sent it (no path normalisation,
/// so `a//b`, `a/./b` and `a/b` stay three different keys).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectKey(String);

impl ObjectKey {
    /// Checks `key` against the length limit.
    ///
    /// # Errors
    /// [`Error::EmptyObjectKey`] or [`Error::ObjectKeyTooLong`].
    pub fn new(key: &str) -> Result<Self> {
        if key.is_empty() {
            return Err(Error::EmptyObjectKey);
        }
        if key.len() > OBJECT_KEY_MAX_LEN {
            return Err(Error::ObjectKeyTooLong { len: key.len() });
        }
        Ok(Self(key.to_owned()))
    }

    /// The key as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::ObjectKey;
    use crate::Error;

    #[test]
    fn a_key_up_to_1024_bytes_is_kept_byte_for_byte() {
        for key in [
            "a",
            "a//b",
            "a/./b",
            "../x",
            "dir/",
            "ключ €",
            &"k".repeat(1024),
        ] {
            assert_eq!(
                ObjectKey::new(key).map(|k| k.as_str().to_owned()),
                Ok(key.to_owned()),
                "{key}"
            );
        }
    }

    #[test]
    fn the_limit_is_bytes_not_characters() {
        let key = "é".repeat(513);
        assert_eq!(
            ObjectKey::new(&key),
            Err(Error::ObjectKeyTooLong { len: 1026 })
        );
    }

    #[test]
    fn an_empty_or_over_long_key_is_refused() {
        assert_eq!(ObjectKey::new(""), Err(Error::EmptyObjectKey));
        assert_eq!(
            ObjectKey::new(&"k".repeat(1025)),
            Err(Error::ObjectKeyTooLong { len: 1025 })
        );
    }
}
