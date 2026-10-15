//! Sealing access-key secrets: the ONE place a cipher is built, a nonce drawn and associated data assembled.
//!
//! The root key ([`IamKey`]) never encrypts. Each space has its own key, derived from the root with HKDF-SHA256 and
//! labelled with the space, so a record that reaches the wrong space opens under the wrong key. A secret is sealed
//! with XChaCha20-Poly1305 under a random 192-bit nonce: nodes seal concurrently and a restored store can rewind any
//! counter, and at this nonce size a random draw needs no per-key bound. The associated data binds the ciphertext to
//! the access key id, the user, the space and the root that sealed it; the caller rebuilds it from where the record
//! lives, never from the record.

use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{Key, KeyInit, XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use sha2::{Digest, Sha256};
use tessari_s3_types::{IamKey, SecretKey};
use zeroize::{Zeroize, Zeroizing};

use crate::{Error, Result};

/// The algorithm every sealed secret names; anything else does not open.
pub(crate) const ALGORITHM: &str = "xchacha20poly1305";

/// Labels a per-space key derivation; the space's name follows it.
const KEK_LABEL: &[u8] = b"tessaridb-s3/access-key-kek/v1\0";
/// Labels the hash that names a root key.
const KEK_ID_LABEL: &[u8] = b"tessaridb-s3/iam-key-id/v1\0";
/// Opens the associated data of every sealed secret.
const AAD_LABEL: &[u8] = b"tessaridb-s3/access-key/v1\0";
/// Precedes a one-key credential's scope in its associated data; a full key's associated data ends before it.
const SCOPE_LABEL: &[u8] = b"\0upload\0";
/// XChaCha20's nonce length.
const NONCE_LEN: usize = 24;

/// A secret as stored: what is needed to open it, none of it secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Sealed {
    /// The ciphertext followed by its 16-byte tag.
    pub ciphertext: Vec<u8>,
    /// The 24-byte nonce it was sealed under.
    pub nonce: Vec<u8>,
    /// Which root key sealed it.
    pub kek_id: String,
    /// Which algorithm sealed it.
    pub algorithm: String,
}

/// Where a sealed secret belongs: rebuilt from the record's own id and the user's record, never read from the
/// sealed record.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Binding<'a> {
    /// The access key id the secret belongs to.
    pub key_id: &'a str,
    /// The user holding the key.
    pub user: &'a str,
    /// The user's space.
    pub space: &'a str,
    /// What a one-key credential may write, from the record's own fields; `None` for a full key.
    pub scope: Option<ScopeBinding<'a>>,
}

/// A one-key credential's scope as bound into its associated data, so a record whose scope was stripped or widened
/// does not open — rather than opening as a broader key.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ScopeBinding<'a> {
    /// The bucket it may write.
    pub bucket: &'a str,
    /// The key it may write.
    pub key: &'a str,
    /// When it expires, in seconds since the epoch.
    pub expires: i64,
}

/// Seals and opens access-key secrets under one root key.
pub(crate) struct Sealer {
    root: IamKey,
    kek_id: String,
}

impl Sealer {
    /// A sealer over `root`.
    pub(crate) fn new(root: IamKey) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(KEK_ID_LABEL);
        hasher.update(root.expose());
        let kek_id = hasher
            .finalize()
            .iter()
            .take(8)
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Self { root, kek_id }
    }

    /// The cipher under `space`'s key, derived from the root; the derived key is scrubbed when this returns and the
    /// cipher scrubs its copy when dropped.
    fn cipher(&self, space: &str) -> Result<XChaCha20Poly1305> {
        let mut info = Vec::with_capacity(KEK_LABEL.len().saturating_add(space.len()));
        info.extend_from_slice(KEK_LABEL);
        info.extend_from_slice(space.as_bytes());
        let mut kek = Zeroizing::new([0_u8; 32]);
        Hkdf::<Sha256>::new(None, self.root.expose())
            .expand(&info, kek.as_mut_slice())
            .map_err(|_| Error::Unsealable)?;
        Ok(XChaCha20Poly1305::new(Key::from_slice(kek.as_slice())))
    }

    /// The associated data binding a secret to `binding` under this root.
    fn associated_data(&self, binding: Binding<'_>) -> Vec<u8> {
        let mut aad = Vec::with_capacity(128);
        aad.extend_from_slice(AAD_LABEL);
        for part in [binding.key_id, binding.user, binding.space] {
            aad.extend_from_slice(part.as_bytes());
            aad.push(0);
        }
        aad.extend_from_slice(self.kek_id.as_bytes());
        // A bucket name holds no NUL and the expiry is digits, so the key — which may hold anything — goes last.
        if let Some(scope) = binding.scope {
            aad.extend_from_slice(SCOPE_LABEL);
            aad.extend_from_slice(scope.bucket.as_bytes());
            aad.push(0);
            aad.extend_from_slice(scope.expires.to_string().as_bytes());
            aad.push(0);
            aad.extend_from_slice(scope.key.as_bytes());
        }
        aad
    }

    /// `secret` sealed for `binding`.
    ///
    /// # Errors
    /// [`Error::Randomness`] when no nonce could be drawn.
    pub(crate) fn seal(&self, secret: &SecretKey, binding: Binding<'_>) -> Result<Sealed> {
        let mut nonce = [0_u8; NONCE_LEN];
        getrandom::fill(&mut nonce).map_err(|_| Error::Randomness)?;
        let payload = Payload {
            msg: secret.expose().as_bytes(),
            aad: &self.associated_data(binding),
        };
        let ciphertext = self
            .cipher(binding.space)?
            .encrypt(XNonce::from_slice(&nonce), payload)
            .map_err(|_| Error::Unsealable)?;
        Ok(Sealed {
            ciphertext,
            nonce: nonce.to_vec(),
            kek_id: self.kek_id.clone(),
            algorithm: ALGORITHM.to_owned(),
        })
    }

    /// The secret `sealed` holds, when it was sealed for `binding` under this sealer's root.
    ///
    /// # Errors
    /// [`Error::Unsealable`] for every reason it does not open — another root, another binding, a changed byte, an
    /// unknown algorithm — told apart nowhere, so a caller learns nothing from which.
    pub(crate) fn open(&self, sealed: &Sealed, binding: Binding<'_>) -> Result<SecretKey> {
        if sealed.algorithm != ALGORITHM
            || sealed.kek_id != self.kek_id
            || sealed.nonce.len() != NONCE_LEN
        {
            return Err(Error::Unsealable);
        }
        let payload = Payload {
            msg: &sealed.ciphertext,
            aad: &self.associated_data(binding),
        };
        let plaintext = self
            .cipher(binding.space)?
            .decrypt(XNonce::from_slice(&sealed.nonce), payload)
            .map_err(|_| Error::Unsealable)?;
        String::from_utf8(plaintext)
            .map(SecretKey::new)
            .map_err(|refused| {
                refused.into_bytes().zeroize();
                Error::Unsealable
            })
    }
}

#[cfg(test)]
#[path = "sealer_tests.rs"]
mod tests;
