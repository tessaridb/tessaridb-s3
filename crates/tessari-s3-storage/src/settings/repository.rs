//! What resolving the settings needs from wherever they are kept.

use std::collections::HashMap;
use std::future::Future;

use zeroize::Zeroizing;

use crate::Result;
use crate::keys::Passphrase;

/// The stored settings, by record id.
pub(crate) trait SettingsStore: Send + Sync {
    /// Every plain setting the store holds, by record id.
    fn settings(&self) -> impl Future<Output = Result<HashMap<String, String>>> + Send;
    /// Stores plain setting `id`; one already stored — another node seeded it first — is left as it is.
    fn seed_setting(&self, id: &str, value: &str) -> impl Future<Output = Result<()>> + Send;
    /// Opens the secrets vault with `passphrase` — declaring it when absent — and answers every secret it holds, by
    /// record id. The vault is sealed again before this answers.
    fn secrets(
        &self,
        passphrase: &Passphrase,
    ) -> impl Future<Output = Result<HashMap<String, Zeroizing<String>>>> + Send;
    /// Opens the secrets vault with `passphrase` and stores secret `id`; one already stored is left as it is. The vault
    /// is sealed again before this answers.
    fn seed_secret(
        &self,
        passphrase: &Passphrase,
        id: &str,
        value: &str,
    ) -> impl Future<Output = Result<()>> + Send;
}
