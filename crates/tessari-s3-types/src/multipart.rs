//! [`UploadId`] and [`PartNumber`]: the handles of a multipart upload and its parts.

use tessari_s3_constants::MULTIPART_MAX_PARTS;

use crate::{Error, Result};

/// A multipart upload's id: 32 lowercase hex digits (a random 128-bit value), opaque to clients.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UploadId([u8; 16]);

impl UploadId {
    /// The id for these 16 bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Reads an id a client sent back.
    ///
    /// # Errors
    /// [`Error::InvalidUploadId`] for anything but 32 lowercase hex digits.
    pub fn parse(text: &str) -> Result<Self> {
        let digits = text.as_bytes();
        if digits.len() != 32 {
            return Err(Error::InvalidUploadId);
        }
        let nibble = |digit: u8| match digit {
            b'0'..=b'9' => Ok(digit.wrapping_sub(b'0')),
            b'a'..=b'f' => Ok(digit.wrapping_sub(b'a').wrapping_add(10)),
            _ => Err(Error::InvalidUploadId),
        };
        let (pairs, _) = digits.as_chunks::<2>();
        let mut bytes = [0_u8; 16];
        for (byte, [high, low]) in bytes.iter_mut().zip(pairs) {
            *byte = (nibble(*high)? << 4) | nibble(*low)?;
        }
        Ok(Self(bytes))
    }

    /// The 16 bytes.
    #[must_use]
    pub const fn bytes(self) -> [u8; 16] {
        self.0
    }

    /// The id as clients see it.
    #[must_use]
    pub fn hex(self) -> String {
        use std::fmt::Write as _;
        self.0
            .iter()
            .fold(String::with_capacity(32), |mut out, byte| {
                let _ = write!(out, "{byte:02x}");
                out
            })
    }
}

/// A part number, 1 to 10,000.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PartNumber(u16);

impl PartNumber {
    /// Reads a part number a client sent.
    ///
    /// # Errors
    /// [`Error::InvalidPartNumber`] for anything but an integer from 1 to 10,000.
    pub fn parse(text: &str) -> Result<Self> {
        if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
            return Err(Error::InvalidPartNumber);
        }
        text.parse::<u16>()
            .map_err(|_| Error::InvalidPartNumber)
            .and_then(Self::new)
    }

    /// The part number `number`.
    ///
    /// # Errors
    /// [`Error::InvalidPartNumber`] outside 1 to 10,000.
    pub fn new(number: u16) -> Result<Self> {
        if (1..=MULTIPART_MAX_PARTS).contains(&number) {
            Ok(Self(number))
        } else {
            Err(Error::InvalidPartNumber)
        }
    }

    /// The number.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::{PartNumber, UploadId};
    use crate::Error;

    #[test]
    fn an_upload_id_round_trips_through_its_hex_form_and_nothing_else_parses() {
        let id = UploadId::from_bytes([0xab; 16]);
        assert_eq!(id.hex(), "ab".repeat(16));
        assert_eq!(UploadId::parse(&id.hex()), Ok(id));
        for bad in [
            "",
            "AB".repeat(16).as_str(),
            &"ab".repeat(15),
            &"ab".repeat(17),
            &"zz".repeat(16),
            "../x",
        ] {
            assert_eq!(UploadId::parse(bad), Err(Error::InvalidUploadId), "{bad}");
        }
    }

    #[test]
    fn part_numbers_run_from_one_to_ten_thousand() {
        assert_eq!(PartNumber::parse("1").map(PartNumber::get), Ok(1));
        assert_eq!(PartNumber::parse("10000").map(PartNumber::get), Ok(10_000));
        for bad in ["0", "10001", "-1", "", "1.0", "+1", " 1", "65537"] {
            assert_eq!(
                PartNumber::parse(bad),
                Err(Error::InvalidPartNumber),
                "{bad:?}"
            );
        }
    }
}
