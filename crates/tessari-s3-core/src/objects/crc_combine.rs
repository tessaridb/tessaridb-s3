//! The CRC of two byte strings joined, from the CRCs of each and the second one's length — no byte is read again.
//!
//! A CRC is the remainder of the message polynomial modulo the generator, so appending `n` bytes multiplies the first
//! CRC by `x^(8n)` modulo the generator before the second CRC is added (XOR). The three CRCs S3 combines are
//! reflected, with the same all-ones initial value and final XOR, which cancel in that identity; arithmetic here is in
//! the reflected bit order, where the top bit of the width holds the `x^0` coefficient.

use super::checksum::ChecksumAlgorithm;

/// The reflected generator polynomial and the width's top bit of a combinable CRC.
const fn parameters(algorithm: ChecksumAlgorithm) -> Option<(u64, u64)> {
    match algorithm {
        ChecksumAlgorithm::Crc32 => Some((0xEDB8_8320, 1 << 31)),
        ChecksumAlgorithm::Crc32c => Some((0x82F6_3B78, 1 << 31)),
        ChecksumAlgorithm::Crc64Nvme => Some((0x9A6C_9329_AC4B_C9B5, 1 << 63)),
        ChecksumAlgorithm::Sha1 | ChecksumAlgorithm::Sha256 => None,
    }
}

/// `a · b` modulo the generator, both in reflected form.
fn multiply(a: u64, mut b: u64, polynomial: u64, top: u64) -> u64 {
    let mut product = 0;
    let mut bit = top;
    while bit != 0 {
        if a & bit != 0 {
            product ^= b;
        }
        // Multiply `b` by x: a right shift in reflected form, reduced when the x^(width-1) term falls off.
        b = if b & 1 == 0 {
            b >> 1
        } else {
            (b >> 1) ^ polynomial
        };
        bit >>= 1;
    }
    product
}

/// `x^(8 · bytes)` modulo the generator, by squaring.
fn shift_of(bytes: u64, polynomial: u64, top: u64) -> u64 {
    let mut result = top;
    let mut square = top >> 8;
    let mut remaining = bytes;
    while remaining != 0 {
        if remaining & 1 == 1 {
            result = multiply(result, square, polynomial, top);
        }
        square = multiply(square, square, polynomial, top);
        remaining >>= 1;
    }
    result
}

/// The CRC of `A ‖ B` from `crc(A)`, `crc(B)` and `B`'s length; `None` for an algorithm that is not a CRC.
#[must_use]
pub fn combine(
    algorithm: ChecksumAlgorithm,
    first: u64,
    second: u64,
    second_len: u64,
) -> Option<u64> {
    let (polynomial, top) = parameters(algorithm)?;
    Some(
        multiply(
            shift_of(second_len, polynomial, top),
            first,
            polynomial,
            top,
        ) ^ second,
    )
}
