//! Reading a data file back: one block at a time on the blocking pool, verified against its hash before a byte of it
//! is returned.

use std::os::unix::fs::FileExt;
use std::sync::Arc;

use super::super::format::{HASH_LEN, block_hash, block_span};
use super::{DataReader, blocking, io};
use crate::{Error, Result};

impl DataReader {
    /// The object's size in bytes.
    #[must_use]
    pub const fn size(&self) -> u64 {
        self.size
    }

    /// Bytes per block.
    #[must_use]
    pub const fn block_size(&self) -> u32 {
        self.block_size
    }

    /// Block `index`, verified against its hash before a byte of it is returned.
    ///
    /// # Errors
    /// [`Error::Corrupt`] when the block fails its hash, [`Error::DataIo`] when the drive fails.
    pub async fn read_block(&self, index: u64) -> Result<Vec<u8>> {
        let corrupt = |reason| Error::Corrupt {
            id: self.hex.clone(),
            reason,
        };
        let (offset, length) =
            block_span(index, self.size, self.block_size).ok_or_else(|| corrupt("block index"))?;
        let length = usize::try_from(length).map_err(|_| corrupt("block length"))?;
        let file = Arc::clone(&self.file);
        let (block, verified) = blocking("read", move || {
            let mut framed = vec![0; length.saturating_add(HASH_LEN)];
            file.read_exact_at(&mut framed, offset)
                .map_err(io("read"))?;
            let stored = framed.split_off(length);
            let verified = block_hash(index, &framed)[..] == stored[..];
            Ok((framed, verified))
        })
        .await?;
        if verified {
            Ok(block)
        } else {
            tracing::error!(data = %self.hex, index, "a data block failed its hash");
            Err(corrupt("block hash"))
        }
    }
}
