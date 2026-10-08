//! [`ObjectReader`]: the bytes of a stored object, read block by block whether they sit in one data file on this
//! node or in erasure-coded shards across the cluster.

use crate::Result;
use crate::data::DataReader;
use crate::erasure::ErasureReader;

/// An object's data, open for reading; every block is verified before a byte of it is returned.
pub enum ObjectReader {
    /// One data file on this node.
    File(DataReader),
    /// Shards across the cluster; a block is one stripe.
    Erasure(ErasureReader),
}

impl ObjectReader {
    /// The object's size in bytes.
    #[must_use]
    pub const fn size(&self) -> u64 {
        match self {
            Self::File(reader) => reader.size(),
            Self::Erasure(reader) => reader.size(),
        }
    }

    /// Bytes per block.
    #[must_use]
    pub const fn block_size(&self) -> u32 {
        match self {
            Self::File(reader) => reader.block_size(),
            Self::Erasure(reader) => reader.block_size(),
        }
    }

    /// Block `index`, verified.
    ///
    /// # Errors
    /// The data file's or the shards' failure.
    pub async fn read_block(&self, index: u64) -> Result<Vec<u8>> {
        match self {
            Self::File(reader) => reader.read_block(index).await,
            Self::Erasure(reader) => reader.read_block(index).await,
        }
    }
}
