//! [`SecretKey`]: a secret access key that cannot be printed and is scrubbed when dropped.

use zeroize::{Zeroize, ZeroizeOnDrop};

/// A secret access key. SigV4 is symmetric, so the server holds it to compute HMACs; it is scrubbed on drop and its
/// `Debug` shows nothing.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SecretKey(String);

impl SecretKey {
    /// Wraps a secret access key.
    #[must_use]
    pub const fn new(secret: String) -> Self {
        Self(secret)
    }

    /// The secret itself, for computing an HMAC. Never log or serialize it.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretKey(..)")
    }
}
