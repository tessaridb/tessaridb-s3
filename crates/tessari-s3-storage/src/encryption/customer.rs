//! Server-side encryption with a customer-provided key (SSE-C). The customer's key is never stored: an object or a
//! multipart upload keeps a random salt and a verifier, and its data keys are wrapped under a key-encryption key
//! derived from the customer's key and that salt — so the stored record alone opens nothing, and a request bringing
//! another key is told apart by the verifier, compared in constant time, before anything is unwrapped.

use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use super::KeyEncryptionKey;
use crate::{Error, Result};

/// The key-encryption-key id every data key wrapped under a customer's key names.
pub const CUSTOMER_KEK_ID: &str = "customer";
/// Separates the derived wrapping key from the derived verifying key.
const WRAP_INFO: &[u8] = b"tessaridb-s3/customer-kek/v1";
const VERIFY_INFO: &[u8] = b"tessaridb-s3/customer-verifier/v1";
/// What the verifier authenticates.
const VERIFY_LABEL: &[u8] = b"tessaridb-s3/customer-key";

/// A customer's 256-bit key, as one request brought it. Never printed, never stored, zeroed when dropped.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct CustomerKey([u8; 32]);

impl CustomerKey {
    /// The key `bytes`.
    #[must_use]
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// `info`'s key derived from this key and `salt`.
    fn derive(&self, salt: &[u8; 32], info: &[u8]) -> Result<Zeroizing<[u8; 32]>> {
        let mut derived = Zeroizing::new([0_u8; 32]);
        Hkdf::<Sha256>::new(Some(salt), &self.0)
            .expand(info, derived.as_mut())
            .map_err(|_| Error::Unsealable)?;
        Ok(derived)
    }
}

impl std::fmt::Debug for CustomerKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CustomerKey(..)")
    }
}

/// What an object or an upload keeps about the customer's key: a salt and a verifier, neither of which opens it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomerSeal {
    /// Drawn fresh for this object or upload.
    pub salt: [u8; 32],
    /// HMAC-SHA256 of a fixed label under the key derived for verifying.
    pub verifier: [u8; 32],
}

impl CustomerSeal {
    /// A fresh salt, and the verifier of `key` under it.
    ///
    /// # Errors
    /// [`Error::Randomness`] when no salt could be drawn; [`Error::Unsealable`] when a derivation refuses.
    pub fn new(key: &CustomerKey) -> Result<Self> {
        let mut salt = [0_u8; 32];
        getrandom::fill(&mut salt).map_err(|_| Error::Randomness)?;
        let verifier = verifier(key, &salt)?;
        Ok(Self { salt, verifier })
    }

    /// Whether `key` is the key this seal was made for; compared in constant time.
    ///
    /// # Errors
    /// [`Error::Unsealable`] when a derivation refuses.
    pub fn matches(&self, key: &CustomerKey) -> Result<bool> {
        Ok(verifier(key, &self.salt)?.ct_eq(&self.verifier).into())
    }

    /// The key-encryption key `key` gives under this seal's salt.
    ///
    /// # Errors
    /// [`Error::Unsealable`] when the derivation refuses.
    pub fn kek(&self, key: &CustomerKey) -> Result<KeyEncryptionKey> {
        Ok(KeyEncryptionKey::new(
            CUSTOMER_KEK_ID.to_owned(),
            key.derive(&self.salt, WRAP_INFO)?,
        ))
    }
}

fn verifier(key: &CustomerKey, salt: &[u8; 32]) -> Result<[u8; 32]> {
    let verifying = key.derive(salt, VERIFY_INFO)?;
    let mut mac =
        <Hmac<Sha256> as Mac>::new_from_slice(verifying.as_ref()).map_err(|_| Error::Unsealable)?;
    mac.update(VERIFY_LABEL);
    Ok(mac.finalize().into_bytes().into())
}

#[cfg(test)]
#[path = "customer_tests.rs"]
mod tests;
