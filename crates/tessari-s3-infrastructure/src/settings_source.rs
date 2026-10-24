//! The stored settings and the environment, resolved into the one view the configuration is read from.
//!
//! The metadata store is a second source of the same variables, consulted before the environment, so every check,
//! default and refusal-by-name in [`crate::S3Config::from_lookup`] applies to a stored value exactly as to one from
//! the environment.

use std::collections::HashMap;

use crate::settings_keys::{Class, STORED, stored_setting};
use crate::{Error, Result};

/// The stored-class settings as resolved, with what the store must be given and what the environment contradicts.
#[derive(Debug, Default)]
pub struct Resolved {
    values: HashMap<&'static str, String>,
    seed: Vec<(&'static str, String)>,
    differing: Vec<&'static str>,
}

impl Resolved {
    /// The resolved value of stored setting `key`, or `None` when neither source holds one (or `key` is not stored).
    #[must_use]
    pub fn value(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    /// The settings the environment holds and the store does not: written to the store before the node serves.
    #[must_use]
    pub fn seed(&self) -> Vec<(&'static str, &str)> {
        self.seed
            .iter()
            .map(|(key, value)| (*key, value.as_str()))
            .collect()
    }

    /// The settings whose environment value differs from the stored one, which wins; named, never shown.
    #[must_use]
    pub fn differing(&self) -> Vec<&'static str> {
        self.differing.clone()
    }

    /// `key` as the configuration reads it: the resolved value of a stored setting, else the environment's.
    pub fn lookup(&self, env: &impl Fn(&str) -> Option<String>, key: &str) -> Option<String> {
        match stored_setting(key) {
            Some(_) => self.value(key).map(str::to_owned),
            None => env(key),
        }
    }
}

/// Resolves every stored setting from `held` (what the store holds, by variable name) and `env`.
///
/// A secret is taken from the store, and seeded into it, only when `secrets_open` — the store's secrets vault is
/// opened with a passphrase from the environment; otherwise it stays an environment value, as before secrets were
/// stored, because a node that must be unsealed by hand cannot read the root secret its operator signs in with.
///
/// # Errors
/// [`Error::InvalidConfig`] naming a fixed setting (the erasure code, the cluster secret) whose environment value
/// differs from the stored one: a cluster runs one code and one secret, and the store holds the one it runs.
pub fn resolve(
    env: &impl Fn(&str) -> Option<String>,
    held: &HashMap<String, String>,
    secrets_open: bool,
) -> Result<Resolved> {
    let mut resolved = Resolved::default();
    for stored in &STORED {
        let from_env = env(stored.key).filter(|value| !value.trim().is_empty());
        if stored.class == Class::Secret && !secrets_open {
            if let Some(value) = from_env {
                resolved.values.insert(stored.key, value);
            }
            continue;
        }
        match (held.get(stored.key), from_env) {
            (Some(kept), Some(given)) => {
                if *kept != given {
                    if stored.fixed {
                        return Err(Error::InvalidConfig {
                            key: stored.key,
                            reason: "differs from the value the metadata store holds, which may not change once stored",
                        });
                    }
                    resolved.differing.push(stored.key);
                }
                resolved.values.insert(stored.key, kept.clone());
            }
            (Some(kept), None) => {
                resolved.values.insert(stored.key, kept.clone());
            }
            (None, Some(given)) => {
                resolved.seed.push((stored.key, given.clone()));
                resolved.values.insert(stored.key, given);
            }
            (None, None) => {}
        }
    }
    Ok(resolved)
}

#[cfg(test)]
#[path = "settings_source_tests.rs"]
mod tests;
