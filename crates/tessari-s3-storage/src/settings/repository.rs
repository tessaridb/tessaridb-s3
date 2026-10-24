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
    /// Sets plain setting `id` to `value` only while it holds `expected` (`None`: only while it is absent); `false`
    /// when it held something else, which the caller reports instead of overwriting.
    fn change_setting(
        &self,
        id: &str,
        expected: Option<&str>,
        value: &str,
    ) -> impl Future<Output = Result<bool>> + Send;
    /// The ids of the secrets the vault holds — names only, nothing revealed; empty when no vault is declared.
    fn secret_ids(&self) -> impl Future<Output = Result<Vec<String>>> + Send;
    /// Opens the secrets vault with `passphrase` and sets secret `id` to `value`, stored or not. The vault is sealed
    /// again before this answers.
    fn change_secret(
        &self,
        passphrase: &Passphrase,
        id: &str,
        value: &str,
    ) -> impl Future<Output = Result<()>> + Send;
}
