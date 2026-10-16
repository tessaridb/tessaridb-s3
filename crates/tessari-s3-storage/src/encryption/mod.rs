//! Server-side encryption of object bytes: the ONE place a cipher is built, a nonce chosen and associated data
//! assembled for object data (design: the S3 scope's G017 plan).
//!
//! Every data file has its own data key, drawn fresh from the platform's generator, and every block of the file is
//! sealed under it with XChaCha20-Poly1305. The block's nonce is its index: one key seals one file, and each index
//! occurs once in it, so no nonce repeats under a key. The associated data names the data file, the block index and
//! whether it is the last block, so a block moved within a file, into another file, or a file cut short at a block
//! boundary does not open. The data key is stored only wrapped — sealed under a key-encryption key with a random
//! 192-bit nonce (keys are wrapped concurrently on many nodes, and a restored store can rewind any counter) and
//! associated data naming that key and the data file. Every reason a seal does not open is the same error.

use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{Key, KeyInit, XChaCha20Poly1305, XNonce};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::{Error, Result};

pub(crate) mod stream;

/// The algorithm every wrapped data key names; anything else does not open.
pub const WRAP_ALGORITHM: &str = "xchacha20poly1305";
/// Bytes a sealed block adds to the block it seals: the Poly1305 tag.
pub const BLOCK_TAG_LEN: usize = 16;
/// XChaCha20's nonce length.
const NONCE_LEN: usize = 24;
/// Opens the associated data of a wrapped data key.
const WRAP_AAD_LABEL: &[u8] = b"tessaridb-s3/data-key/v1\0";
/// Opens the associated data of a sealed block.
const BLOCK_AAD_LABEL: &[u8] = b"tessaridb-s3/data-block/v1\0";
/// The first eight bytes of every block nonce.
const BLOCK_NONCE_LABEL: [u8; 8] = *b"ts3block";

/// One data file's key. Never printed, never serialized, zeroed when dropped.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct DataKey([u8; 32]);

impl DataKey {
    /// A fresh key from the platform's generator.
    ///
    /// # Errors
    /// [`Error::Randomness`] when the generator answers nothing.
    pub fn generate() -> Result<Self> {
        let mut key = [0_u8; 32];
        getrandom::fill(&mut key).map_err(|_| Error::Randomness)?;
        Ok(Self(key))
    }
}

impl std::fmt::Debug for DataKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DataKey(..)")
    }
}

/// A key that wraps data keys, under its id. Its bytes are never printed and are zeroed when it is dropped.
pub struct KeyEncryptionKey {
    id: String,
    key: Zeroizing<[u8; 32]>,
}

impl KeyEncryptionKey {
    /// The key `key`, known as `id`.
    pub fn new(id: String, key: Zeroizing<[u8; 32]>) -> Self {
        Self { id, key }
    }

    /// Its id, which every key it wraps names.
    pub fn id(&self) -> &str {
        &self.id
    }
}

impl std::fmt::Debug for KeyEncryptionKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyEncryptionKey")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// A data key as stored: what is needed to unwrap it, none of it secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrappedKey {
    /// Which algorithm wrapped it.
    pub algorithm: String,
    /// Which key-encryption key wrapped it.
    pub kek_id: String,
    /// The 24-byte nonce it was wrapped under.
    pub nonce: Vec<u8>,
    /// The wrapped key followed by its tag.
    pub ciphertext: Vec<u8>,
}

/// `msg` sealed under `key` and `nonce` with `aad`.
fn aead_seal(key: &[u8; 32], nonce: &[u8; NONCE_LEN], aad: &[u8], msg: &[u8]) -> Result<Vec<u8>> {
    XChaCha20Poly1305::new(Key::from_slice(key))
        .encrypt(XNonce::from_slice(nonce), Payload { msg, aad })
        .map_err(|_| Error::Unsealable)
}

/// What `sealed` holds, when it was sealed under `key`, `nonce` and `aad`.
fn aead_open(
    key: &[u8; 32],
    nonce: &[u8; NONCE_LEN],
    aad: &[u8],
    sealed: &[u8],
) -> Result<Vec<u8>> {
    XChaCha20Poly1305::new(Key::from_slice(key))
        .decrypt(XNonce::from_slice(nonce), Payload { msg: sealed, aad })
        .map_err(|_| Error::Unsealable)
}

/// The nonce of block `index`: the label, the index, then zeros.
fn block_nonce(index: u64) -> [u8; NONCE_LEN] {
    let mut nonce = [0_u8; NONCE_LEN];
    nonce[..8].copy_from_slice(&BLOCK_NONCE_LABEL);
    nonce[8..16].copy_from_slice(&index.to_le_bytes());
    nonce
}

/// The associated data of block `index` of data file `data`.
fn block_aad(data: [u8; 16], index: u64, last: bool) -> Vec<u8> {
    let mut aad = Vec::with_capacity(BLOCK_AAD_LABEL.len().saturating_add(25));
    aad.extend_from_slice(BLOCK_AAD_LABEL);
    aad.extend_from_slice(&data);
    aad.extend_from_slice(&index.to_le_bytes());
    aad.push(u8::from(last));
    aad
}

/// Block `index` of data file `data` sealed under `key`; `last` when no block follows it.
///
/// # Errors
/// [`Error::Unsealable`] when the cipher refuses (a block over its length limit).
pub fn seal_block(
    key: &DataKey,
    data: [u8; 16],
    index: u64,
    last: bool,
    plain: &[u8],
) -> Result<Vec<u8>> {
    aead_seal(
        &key.0,
        &block_nonce(index),
        &block_aad(data, index, last),
        plain,
    )
}

/// The bytes block `index` of data file `data` holds, when it was sealed there under `key`.
///
/// # Errors
/// [`Error::Unsealable`] for every reason it does not open — another key, another place, a changed or missing byte.
pub fn open_block(
    key: &DataKey,
    data: [u8; 16],
    index: u64,
    last: bool,
    sealed: &[u8],
) -> Result<Vec<u8>> {
    aead_open(
        &key.0,
        &block_nonce(index),
        &block_aad(data, index, last),
        sealed,
    )
}

/// The associated data binding a wrapped key to `kek_id` and data file `data`.
fn wrap_aad(kek_id: &str, data: [u8; 16]) -> Vec<u8> {
    let mut aad = Vec::with_capacity(
        WRAP_AAD_LABEL
            .len()
            .saturating_add(kek_id.len())
            .saturating_add(17),
    );
    aad.extend_from_slice(WRAP_AAD_LABEL);
    aad.extend_from_slice(kek_id.as_bytes());
    aad.push(0);
    aad.extend_from_slice(&data);
    aad
}

/// `key` wrapped under `kek` for data file `data`.
///
/// # Errors
/// [`Error::Randomness`] when no nonce could be drawn; [`Error::Unsealable`] when the cipher refuses.
pub fn wrap(kek: &KeyEncryptionKey, key: &DataKey, data: [u8; 16]) -> Result<WrappedKey> {
    let mut nonce = [0_u8; NONCE_LEN];
    getrandom::fill(&mut nonce).map_err(|_| Error::Randomness)?;
    let ciphertext = aead_seal(&kek.key, &nonce, &wrap_aad(&kek.id, data), &key.0)?;
    Ok(WrappedKey {
        algorithm: WRAP_ALGORITHM.to_owned(),
        kek_id: kek.id.clone(),
        nonce: nonce.to_vec(),
        ciphertext,
    })
}

/// The data key `wrapped` holds, when it was wrapped under `kek` for data file `data`.
///
/// # Errors
/// [`Error::Unsealable`] for every reason it does not open — another key, another data file, an algorithm not on the
/// list, a changed byte — told apart nowhere.
pub fn unwrap(kek: &KeyEncryptionKey, wrapped: &WrappedKey, data: [u8; 16]) -> Result<DataKey> {
    if wrapped.algorithm != WRAP_ALGORITHM || wrapped.kek_id != kek.id {
        return Err(Error::Unsealable);
    }
    let nonce: [u8; NONCE_LEN] = wrapped
        .nonce
        .as_slice()
        .try_into()
        .map_err(|_| Error::Unsealable)?;
    let opened = Zeroizing::new(aead_open(
        &kek.key,
        &nonce,
        &wrap_aad(&kek.id, data),
        &wrapped.ciphertext,
    )?);
    let key: [u8; 32] = opened
        .as_slice()
        .try_into()
        .map_err(|_| Error::Unsealable)?;
    Ok(DataKey(key))
}

#[cfg(test)]
mod tests;
