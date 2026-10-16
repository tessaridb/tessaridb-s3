//! What key custody holds: the keys passphrase as checked before anything is sent, and the keys an unsealed node
//! keeps in memory.

use std::collections::BTreeMap;

use zeroize::Zeroizing;

use super::entity::StoredKey;
use crate::encryption::KeyEncryptionKey;
use crate::{Error, Result};

/// Fewest characters (code points) a keys passphrase has — NIST SP 800-63B-4 §3.1.1.2, a single-factor secret is at
/// least 15 (pages.nist.gov/800-63-4/sp800-63b/authenticators, retrieved 2026-10-16).
const PASSPHRASE_MIN_CHARS: usize = 15;
/// Most characters a keys passphrase has.
const PASSPHRASE_MAX_CHARS: usize = 256;
/// Every key-encryption key the server mints is named this, then its number.
pub(crate) const SERVER_KEY_PREFIX: &str = "server_";

/// The passphrase that opens the keys vault. It may hold no `'`, no `\` and no control character: the engine takes
/// it once as a quoted literal when the vault is declared, and such a literal needs no escaping and cannot become
/// syntax. Never printed, zeroed when dropped.
pub struct Passphrase(Zeroizing<String>);

impl Passphrase {
    /// `text`, when it is a usable keys passphrase.
    ///
    /// # Errors
    /// [`Error::WeakPassphrase`] naming what is wrong — never the passphrase itself.
    pub fn new(text: Zeroizing<String>) -> Result<Self> {
        let chars = text.chars().count();
        if chars < PASSPHRASE_MIN_CHARS {
            return Err(Error::WeakPassphrase {
                reason: "shorter than 15 characters",
            });
        }
        if chars > PASSPHRASE_MAX_CHARS {
            return Err(Error::WeakPassphrase {
                reason: "longer than 256 characters",
            });
        }
        if text
            .chars()
            .any(|c| c == '\'' || c == '\\' || c.is_control())
        {
            return Err(Error::WeakPassphrase {
                reason: "holds a quote, a backslash or a control character",
            });
        }
        Ok(Self(text))
    }

    /// The passphrase, for the one call that sends it.
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Passphrase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Passphrase(..)")
    }
}

/// Whether this node can encrypt and decrypt object data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyState {
    /// No keys vault exists yet: the first unseal creates it and the first key.
    Uninitialised,
    /// The vault exists and this node holds no key.
    Sealed,
    /// This node holds the keys; new data keys are wrapped under `active`.
    Unsealed {
        /// The key new data keys are wrapped under.
        active: String,
        /// Every key held.
        keys: usize,
    },
}

/// The key-encryption keys an unsealed node holds.
#[derive(Debug)]
pub struct KeyRing {
    active: KeyEncryptionKey,
    /// Every other key, which still unwraps what was wrapped under it.
    others: BTreeMap<String, KeyEncryptionKey>,
}

/// The number in a server key's id, `None` for any other id.
fn server_number(id: &str) -> Option<u64> {
    id.strip_prefix(SERVER_KEY_PREFIX)?.parse().ok()
}

impl KeyRing {
    /// The ring over `stored`; the active key is the highest-numbered server key.
    ///
    /// # Errors
    /// [`Error::Unsealable`] when it holds no server key — data could be read but nothing new wrapped.
    pub(crate) fn new(stored: Vec<StoredKey>) -> Result<Self> {
        let active_id = stored
            .iter()
            .filter_map(|key| server_number(&key.id).map(|number| (number, key.id.clone())))
            .max()
            .map(|(_, id)| id)
            .ok_or(Error::Unsealable)?;
        let mut active = None;
        let mut others = BTreeMap::new();
        for key in stored {
            let kek = KeyEncryptionKey::new(key.id.clone(), key.key);
            if key.id == active_id {
                active = Some(kek);
            } else {
                others.insert(key.id, kek);
            }
        }
        Ok(Self {
            active: active.ok_or(Error::Unsealable)?,
            others,
        })
    }

    /// The key new data keys are wrapped under.
    #[must_use]
    pub const fn active(&self) -> &KeyEncryptionKey {
        &self.active
    }

    /// The key named `id`, to unwrap a data key that names it.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&KeyEncryptionKey> {
        if self.active.id() == id {
            Some(&self.active)
        } else {
            self.others.get(id)
        }
    }

    /// What the ring says about this node.
    #[must_use]
    pub fn state(&self) -> KeyState {
        KeyState::Unsealed {
            active: self.active.id().to_owned(),
            keys: self.others.len().saturating_add(1),
        }
    }
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
