//! Which key-encryption key seals an object: the server's own active key (SSE-S3), or a key an operator created by
//! name (SSE-KMS). A stored data key names the key that wrapped it, and that id alone says which it was: `server_<n>`
//! or `kms_<name>` — no second field can disagree with it.

use super::WrappedKey;
use crate::keys::SERVER_KEY_PREFIX;

/// Every named key is stored under this prefix, then its name.
pub(crate) const NAMED_KEY_PREFIX: &str = "kms_";
/// Most characters a key name has.
const NAME_MAX: usize = 64;

/// The name of a key an operator created: 1 to 64 of `a-z`, `0-9` and `-`. It is spliced into a record id, so it
/// can never carry anything else.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct KeyName(String);

impl KeyName {
    /// `text`, when it is a usable key name.
    #[must_use]
    pub fn new(text: &str) -> Option<Self> {
        let usable = !text.is_empty()
            && text.len() <= NAME_MAX
            && text
                .bytes()
                .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'-'));
        usable.then(|| Self(text.to_owned()))
    }

    /// The name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The id of the vault record holding this key.
    pub(crate) fn record_id(&self) -> String {
        format!("{NAMED_KEY_PREFIX}{}", self.0)
    }
}

impl std::fmt::Display for KeyName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Which key seals an object's data keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sealing {
    /// The server's own active key (`x-amz-server-side-encryption: AES256`).
    Server,
    /// A key an operator created by name (`aws:kms`).
    Named(KeyName),
}

impl Sealing {
    /// How the data key `wrapped` holds was sealed, read from the key that wrapped it; `None` for an id that is
    /// neither a server key nor a named one.
    #[must_use]
    pub fn of(wrapped: &WrappedKey) -> Option<Self> {
        let id = wrapped.kek_id.as_str();
        if let Some(name) = id.strip_prefix(NAMED_KEY_PREFIX) {
            return KeyName::new(name).map(Self::Named);
        }
        let number = id.strip_prefix(SERVER_KEY_PREFIX)?;
        (!number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()))
            .then_some(Self::Server)
    }
}

#[cfg(test)]
#[path = "sealing_tests.rs"]
mod tests;
