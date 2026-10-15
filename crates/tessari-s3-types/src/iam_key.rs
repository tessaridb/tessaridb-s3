//! [`IamKey`]: the root key access-key secrets are sealed under, held so it cannot be printed and is scrubbed on drop.

use zeroize::{Zeroize, ZeroizeOnDrop};

/// The 32-byte root key from `TESSARIDB_S3_IAM_KEY`. It never encrypts anything itself: per-space keys are derived
/// from it, and those seal the secrets.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct IamKey([u8; 32]);

impl IamKey {
    /// The key written as 64 hexadecimal digits (either case, surrounding whitespace ignored), or `None` for
    /// anything else.
    #[must_use]
    pub fn from_hex(text: &str) -> Option<Self> {
        let digits = text.trim().as_bytes();
        if digits.len() != 64 {
            return None;
        }
        let mut key = Self([0; 32]);
        let (pairs, _) = digits.as_chunks::<2>();
        for (byte, &[high, low]) in key.0.iter_mut().zip(pairs) {
            *byte = hex_value(high)?.checked_shl(4)? | hex_value(low)?;
        }
        Some(key)
    }

    /// The key's bytes, for deriving per-space keys. Never log or serialize them.
    #[must_use]
    pub const fn expose(&self) -> &[u8; 32] {
        &self.0
    }
}

/// The value of one hexadecimal digit.
const fn hex_value(digit: u8) -> Option<u8> {
    match digit {
        b'0'..=b'9' => digit.checked_sub(b'0'),
        b'a'..=b'f' => match digit.checked_sub(b'a') {
            Some(offset) => offset.checked_add(10),
            None => None,
        },
        b'A'..=b'F' => match digit.checked_sub(b'A') {
            Some(offset) => offset.checked_add(10),
            None => None,
        },
        _ => None,
    }
}

impl std::fmt::Debug for IamKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("IamKey(..)")
    }
}

#[cfg(test)]
#[path = "iam_key_tests.rs"]
mod tests;
