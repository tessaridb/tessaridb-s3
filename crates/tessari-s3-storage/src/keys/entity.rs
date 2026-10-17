//! A key-encryption key as the vault holds it.

use zeroize::Zeroizing;

/// One record of the keys vault, revealed.
pub(crate) struct StoredKey {
    /// The record's id: `server_<n>` for a key the server minted.
    pub(crate) id: String,
    /// The 32 key bytes.
    pub(crate) key: Zeroizing<[u8; 32]>,
}
