//! What key custody needs from wherever the keys are kept.

use std::future::Future;

use zeroize::Zeroizing;

use super::entity::StoredKey;
use super::model::Passphrase;
use crate::Result;

/// The keys vault. Nothing here holds a key past the call that returns it.
pub(crate) trait KeyVault: Send + Sync {
    /// Whether the keys vault has been declared.
    fn declared(&self) -> impl Future<Output = Result<bool>> + Send;
    /// Opens the vault with `passphrase` — declaring it, and storing `first` as the first server key, when it holds
    /// none — and answers every key it holds. The vault is sealed again before this answers.
    fn open(
        &self,
        passphrase: &Passphrase,
        first: &Zeroizing<[u8; 32]>,
    ) -> impl Future<Output = Result<Vec<StoredKey>>> + Send;
}
