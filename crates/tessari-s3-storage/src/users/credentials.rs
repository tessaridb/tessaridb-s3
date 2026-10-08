//! Fresh credentials, drawn from the operating system's random source and nothing else.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use tessari_s3_types::SecretKey;
use zeroize::Zeroizing;

use crate::{Error, Result};

/// The characters after an access key id's `TS`: 32 of them, so a random byte's low five bits pick one uniformly.
const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
/// Characters drawn after the prefix; with `TS`, an id is 20 characters, as S3 access key ids are.
const ID_DRAWN: usize = 18;
/// Random bytes behind a secret: 240 bits, written as 40 base64 characters.
const SECRET_BYTES: usize = 30;

/// A new access key id: `TS` and 18 characters of `A-Z2-7`.
///
/// # Errors
/// [`Error::Randomness`] when the random source fails.
pub(crate) fn access_key_id() -> Result<String> {
    let mut drawn = [0_u8; ID_DRAWN];
    getrandom::fill(&mut drawn).map_err(|_| Error::Randomness)?;
    let mut id = String::with_capacity(ID_DRAWN.saturating_add(2));
    id.push_str("TS");
    for byte in drawn {
        let index = usize::from(byte & 0x1f);
        id.push(char::from(*ALPHABET.get(index).ok_or(Error::Randomness)?));
    }
    Ok(id)
}

/// A new secret: 30 random bytes as 40 base64 characters; the bytes are scrubbed once written out.
///
/// # Errors
/// [`Error::Randomness`] when the random source fails.
pub(crate) fn secret() -> Result<SecretKey> {
    let mut bytes = Zeroizing::new([0_u8; SECRET_BYTES]);
    getrandom::fill(bytes.as_mut_slice()).map_err(|_| Error::Randomness)?;
    Ok(SecretKey::new(STANDARD.encode(bytes.as_slice())))
}
