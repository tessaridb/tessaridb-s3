//! Key custody on one node: sealed until an operator presents the keys passphrase, then holding the key-encryption
//! keys in memory until it is sealed again or stops. Each S3 node holds its own; unsealing one unseals only it.

use std::sync::{Arc, PoisonError, RwLock};

use zeroize::Zeroizing;

use super::model::{KeyRing, KeyState, Passphrase};
use super::repository::KeyVault;
use super::tessaridb::TessariKeys;
use crate::encryption::{KeyName, Sealing};
use crate::{Error, Result};

/// The keys this node holds. Written only by an unseal or a seal; read on every encrypted read and write, and no
/// lock is held past cloning the `Arc` out. A poisoned lock is read through: the slot holds a whole value either way.
pub struct KeyService {
    vault: TessariKeys,
    ring: RwLock<Option<Arc<KeyRing>>>,
}

impl KeyService {
    /// Sealed custody over `vault`.
    #[must_use]
    pub fn new(vault: TessariKeys) -> Self {
        Self {
            vault,
            ring: RwLock::new(None),
        }
    }

    /// The keys, when this node holds them.
    ///
    /// # Errors
    /// [`Error::KeysSealed`] when it does not.
    pub fn ring(&self) -> Result<Arc<KeyRing>> {
        self.ring
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .ok_or(Error::KeysSealed)
    }

    /// Refuses `sealing` unless this node can seal under it now: the server's key, or the named key it holds.
    ///
    /// # Errors
    /// [`Error::KeysSealed`] when this node holds no keys; [`Error::UnknownKey`] for a name it does not hold.
    pub fn check(&self, sealing: &Sealing) -> Result<()> {
        let ring = self.ring()?;
        match sealing {
            Sealing::Server => Ok(()),
            Sealing::Named(name) => {
                ring.get(&name.record_id())
                    .map(|_| ())
                    .ok_or_else(|| Error::UnknownKey {
                        name: name.to_string(),
                    })
            }
        }
    }

    /// Whether this node can encrypt, and if not whether the vault exists yet.
    ///
    /// # Errors
    /// The metadata store's refusal or outage, asked only while sealed.
    pub async fn state(&self) -> Result<KeyState> {
        if let Ok(ring) = self.ring() {
            return Ok(ring.state());
        }
        Ok(if self.vault.declared().await? {
            KeyState::Sealed
        } else {
            KeyState::Uninitialised
        })
    }

    /// Opens the keys vault with `passphrase` and holds its keys; on a store with no keys vault, creates it and the
    /// first key.
    ///
    /// # Errors
    /// [`Error::WrongPassphrase`], [`Error::PassphraseThrottled`], [`Error::Randomness`], or the metadata store's
    /// refusal or outage; on any of them this node stays as it was.
    pub async fn unseal(&self, passphrase: &Passphrase) -> Result<KeyState> {
        let mut first = Zeroizing::new([0_u8; 32]);
        getrandom::fill(first.as_mut_slice()).map_err(|_| Error::Randomness)?;
        let ring = Arc::new(KeyRing::new(self.vault.open(passphrase, &first).await?)?);
        let state = ring.state();
        *self.ring.write().unwrap_or_else(PoisonError::into_inner) = Some(ring);
        Ok(state)
    }

    /// Creates a key named `name` — 32 bytes from the platform's generator — in the keys vault, which `passphrase`
    /// opens, then holds the keys again so this node can seal under it at once. Other nodes hold it after their next
    /// unseal.
    ///
    /// # Errors
    /// [`Error::KeyExists`] for a name in use, [`Error::KeysSealed`] before the vault exists (the first unseal creates
    /// it), [`Error::WrongPassphrase`], [`Error::PassphraseThrottled`], [`Error::Randomness`], or the metadata store's.
    pub async fn create_named(&self, passphrase: &Passphrase, name: &KeyName) -> Result<KeyState> {
        let mut key = Zeroizing::new([0_u8; 32]);
        getrandom::fill(key.as_mut_slice()).map_err(|_| Error::Randomness)?;
        self.vault
            .create(passphrase, &name.record_id(), &key)
            .await?;
        self.unseal(passphrase).await
    }

    /// Forgets the keys; encrypted objects cannot be read or written on this node until the next unseal.
    pub fn seal(&self) {
        *self.ring.write().unwrap_or_else(PoisonError::into_inner) = None;
    }
}
