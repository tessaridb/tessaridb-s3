//! Stripes: an object is read in stripes of a fixed size, each stripe is cut into k blocks, and m parity blocks are
//! computed over them. Block j of every stripe belongs to shard j, so a shard is a run of equal-sized blocks and a
//! reader with any k shards can rebuild any stripe.

use super::Code;

/// Why a stripe could not be encoded or decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StripeError {
    /// A stripe size of zero, a stripe empty or longer than the stripe size, or a length past the blocks.
    #[error("a stripe of that size does not fit this geometry")]
    Size,
    /// Not k + m block slots, or a block of the wrong length.
    #[error("the blocks do not match this code's geometry")]
    Shape,
    /// Fewer than k blocks present.
    #[error("{have} shards present where {need} are needed")]
    TooFewShards {
        /// Blocks present.
        have: usize,
        /// Blocks a decode needs (k).
        need: usize,
    },
}

/// The geometry and codec of one erasure code at one stripe size.
#[derive(Debug)]
pub struct Stripes {
    code: Code,
    stripe_size: u32,
    block_len: usize,
}

impl Stripes {
    /// Stripes of `stripe_size` bytes under `code`.
    ///
    /// # Errors
    /// [`StripeError::Size`] for a stripe size of zero.
    pub fn new(code: Code, stripe_size: u32) -> Result<Self, StripeError> {
        if stripe_size == 0 {
            return Err(StripeError::Size);
        }
        // The codec works on 16-bit symbols, so a block is an even number of bytes.
        let block_len = stripe_size
            .div_ceil(u32::from(code.data()))
            .checked_next_multiple_of(2)
            .and_then(|len| usize::try_from(len).ok())
            .ok_or(StripeError::Size)?;
        Ok(Self {
            code,
            stripe_size,
            block_len,
        })
    }

    /// The bytes of one block: the stripe size divided by k, rounded up to an even count.
    #[must_use]
    pub const fn block_len(&self) -> usize {
        self.block_len
    }

    /// The stripes an object of `size` bytes is cut into.
    #[must_use]
    pub fn stripe_count(&self, size: u64) -> u64 {
        size.div_ceil(u64::from(self.stripe_size))
    }

    /// The k + m blocks of one stripe (the last one may be short; it is padded with zeros).
    ///
    /// # Errors
    /// [`StripeError::Size`] for an empty stripe or one longer than the stripe size.
    pub fn encode(&self, stripe: &[u8]) -> Result<Vec<Vec<u8>>, StripeError> {
        let fits = u32::try_from(stripe.len()).is_ok_and(|len| len <= self.stripe_size);
        if stripe.is_empty() || !fits {
            return Err(StripeError::Size);
        }
        let mut blocks = vec![vec![0; self.block_len]; usize::from(self.code.width())];
        for (block, bytes) in blocks.iter_mut().zip(stripe.chunks(self.block_len)) {
            if let Some(head) = block.get_mut(..bytes.len()) {
                head.copy_from_slice(bytes);
            }
        }
        let data = usize::from(self.code.data());
        let parity = reed_solomon_simd::encode(
            data,
            usize::from(self.code.parity()),
            blocks.iter().take(data),
        )
        .map_err(|_| StripeError::Shape)?;
        for (slot, block) in blocks.iter_mut().skip(data).zip(parity) {
            *slot = block;
        }
        Ok(blocks)
    }

    /// The first `len` bytes of the stripe from k + m block slots, any k of them present. Present blocks are
    /// believed — verify them before handing them here.
    ///
    /// # Errors
    /// [`StripeError::Shape`], [`StripeError::Size`] or [`StripeError::TooFewShards`].
    pub fn decode(
        &self,
        blocks: &mut [Option<Vec<u8>>],
        len: usize,
    ) -> Result<Vec<u8>, StripeError> {
        let data = usize::from(self.code.data());
        let wrong_block = blocks
            .iter()
            .flatten()
            .any(|block| block.len() != self.block_len);
        if blocks.len() != usize::from(self.code.width()) || wrong_block {
            return Err(StripeError::Shape);
        }
        let fits = u32::try_from(len).is_ok_and(|len| len <= self.stripe_size);
        if !fits {
            return Err(StripeError::Size);
        }
        let have = blocks.iter().filter(|block| block.is_some()).count();
        if have < data {
            return Err(StripeError::TooFewShards { have, need: data });
        }
        self.rebuild_data(blocks)?;
        let mut stripe = Vec::with_capacity(len);
        for block in blocks.iter().take(data).flatten() {
            let wanted = len.saturating_sub(stripe.len()).min(block.len());
            stripe.extend_from_slice(block.get(..wanted).unwrap_or_default());
        }
        Ok(stripe)
    }

    /// Fills every missing data block from the blocks present; parity blocks are left as they are.
    fn rebuild_data(&self, blocks: &mut [Option<Vec<u8>>]) -> Result<(), StripeError> {
        let data = usize::from(self.code.data());
        if blocks.iter().take(data).all(Option::is_some) {
            return Ok(());
        }
        let present = |range: std::ops::Range<usize>| {
            blocks
                .iter()
                .enumerate()
                .skip(range.start)
                .take(range.len())
                .filter_map(|(index, block)| {
                    Some((index.checked_sub(range.start)?, block.as_deref()?))
                })
                .collect::<Vec<_>>()
        };
        let width = usize::from(self.code.width());
        let restored = reed_solomon_simd::decode(
            data,
            usize::from(self.code.parity()),
            present(0..data),
            present(data..width),
        )
        .map_err(|_| StripeError::Shape)?;
        for (index, block) in restored {
            if let Some(slot) = blocks.get_mut(index) {
                *slot = Some(block);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "stripe_tests.rs"]
mod tests;
