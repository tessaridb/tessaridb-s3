//! [`UploadId`] and [`PartNumber`]: the handles of a multipart upload and its parts.

use tessari_s3_constants::MULTIPART_MAX_PARTS;

use crate::{Error, Result, Timestamp};

const NANOS_PER_SECOND: u64 = 1_000_000_000;

/// A multipart upload's id: a random 128-bit value. Clients see it as 48 lowercase hex digits — the initiation
/// instant in nanoseconds (16) and then the value (32) — so one key's ids sort as its uploads were initiated, which is
/// what ListMultipartUploads' `upload-id-marker` compares; an earlier build's 32-digit form is still accepted.
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
    /// [`Error::InvalidUploadId`] for anything but 48 or 32 lowercase hex digits.
    pub fn parse(text: &str) -> Result<Self> {
        Self::parse_with_time(text).map(|(id, _)| id)
    }

    /// Reads an id a client sent back, with the initiation instant its 48-digit form carries (none in the 32-digit
    /// form an earlier build issued).
    ///
    /// # Errors
    /// [`Error::InvalidUploadId`] for anything but 48 or 32 lowercase hex digits.
    pub fn parse_with_time(text: &str) -> Result<(Self, Option<Timestamp>)> {
        let digits = text.as_bytes();
        let (time, value) = match digits.len() {
            48 => digits.split_at(16),
            32 => (&[] as &[u8], digits),
            _ => return Err(Error::InvalidUploadId),
        };
        let nibble = |digit: u8| match digit {
            b'0'..=b'9' => Ok(digit.wrapping_sub(b'0')),
            b'a'..=b'f' => Ok(digit.wrapping_sub(b'a').wrapping_add(10)),
            _ => Err(Error::InvalidUploadId),
        };
        let (pairs, _) = value.as_chunks::<2>();
        let mut bytes = [0_u8; 16];
        for (byte, [high, low]) in bytes.iter_mut().zip(pairs) {
            *byte = (nibble(*high)? << 4) | nibble(*low)?;
        }
        let initiated = if time.is_empty() {
            None
        } else {
            let nanos = time.iter().try_fold(0_u64, |total, digit| {
                Ok::<u64, Error>((total << 4) | u64::from(nibble(*digit)?))
            })?;
            let seconds =
                i64::try_from(nanos / NANOS_PER_SECOND).map_err(|_| Error::InvalidUploadId)?;
            let nanos =
                u32::try_from(nanos % NANOS_PER_SECOND).map_err(|_| Error::InvalidUploadId)?;
            Some(Timestamp { seconds, nanos })
        };
        Ok((Self(bytes), initiated))
    }

    /// The 16 bytes.
    #[must_use]
    pub const fn bytes(self) -> [u8; 16] {
        self.0
    }

    /// The 32 hex digits of the value alone — the key the store files the upload under.
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

    /// The id as clients see it, for an upload initiated at `initiated`.
    ///
    /// # Errors
    /// [`Error::InvalidUploadId`] for an instant before 1970 or past what 64 bits of nanoseconds hold (year 2554).
    pub fn wire(self, initiated: Timestamp) -> Result<String> {
        let nanos = u64::try_from(initiated.seconds)
            .ok()
            .and_then(|seconds| seconds.checked_mul(NANOS_PER_SECOND))
            .and_then(|nanos| nanos.checked_add(u64::from(initiated.nanos)))
            .ok_or(Error::InvalidUploadId)?;
        Ok(format!("{nanos:016x}{}", self.hex()))
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
    use crate::{Error, Timestamp};

    #[test]
    fn the_wire_form_carries_the_initiation_instant_and_one_keys_ids_sort_by_it() {
        let earlier = Timestamp {
            seconds: 1_760_000_000,
            nanos: 999_999_999,
        };
        let later = Timestamp {
            seconds: 1_760_000_001,
            nanos: 0,
        };
        // A larger random value first, so only the time prefix can put them in order.
        let first = UploadId::from_bytes([0xff; 16])
            .wire(earlier)
            .expect("in range");
        let second = UploadId::from_bytes([0x00; 16])
            .wire(later)
            .expect("in range");
        assert_eq!(first.len(), 48);
        assert!(first < second, "{first} sorts before {second}");
        assert_eq!(
            UploadId::parse_with_time(&first),
            Ok((UploadId::from_bytes([0xff; 16]), Some(earlier)))
        );
        assert_eq!(
            UploadId::parse(&second),
            Ok(UploadId::from_bytes([0x00; 16]))
        );
        // An earlier build's 32-digit id still names its upload, with no instant.
        let legacy = UploadId::from_bytes([0xab; 16]).hex();
        assert_eq!(
            UploadId::parse_with_time(&legacy),
            Ok((UploadId::from_bytes([0xab; 16]), None))
        );
        let before_1970 = Timestamp {
            seconds: -1,
            nanos: 0,
        };
        assert_eq!(
            UploadId::from_bytes([1; 16]).wire(before_1970),
            Err(Error::InvalidUploadId)
        );
    }

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
