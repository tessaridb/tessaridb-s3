//! [`ObjectReader`]: the bytes of a stored object, read block by block whether they sit in one data file on this
//! node or in erasure-coded shards across the cluster, and opened block by block when they were sealed.

use crate::Result;
use crate::data::DataReader;
use crate::encryption::DataKey;
use crate::encryption::stream::Opener;
use crate::erasure::ErasureReader;

/// An object's data, open for reading; every block is verified before a byte of it is returned.
pub enum ObjectReader {
    /// One data file on this node.
    File(DataReader),
    /// Shards across the cluster; a block is one stripe.
    Erasure(ErasureReader),
    /// Sealed bytes held by another reader; a block is opened before a byte of it is returned.
    Sealed(Box<Sealed>),
}

/// A sealed object's stored bytes and what opens them.
pub struct Sealed {
    stored: ObjectReader,
    opener: Opener,
}

impl ObjectReader {
    /// The `size`-byte object data file `data` holds sealed under `key`, its stored bytes read through `stored`.
    ///
    /// # Errors
    /// [`crate::Error::Unsealable`] when `stored` does not hold what `size` lays out.
    pub(crate) fn sealed(stored: Self, key: DataKey, data: [u8; 16], size: u64) -> Result<Self> {
        let opener = Opener::new(key, data, size, stored.block_size(), stored.size())?;
        Ok(Self::Sealed(Box::new(Sealed { stored, opener })))
    }

    /// The object's size in bytes.
    #[must_use]
    pub fn size(&self) -> u64 {
        match self {
            Self::File(reader) => reader.size(),
            Self::Erasure(reader) => reader.size(),
            Self::Sealed(sealed) => sealed.opener.size(),
        }
    }

    /// Bytes per block.
    #[must_use]
    pub fn block_size(&self) -> u32 {
        match self {
            Self::File(reader) => reader.block_size(),
            Self::Erasure(reader) => reader.block_size(),
            Self::Sealed(sealed) => sealed.opener.block_size(),
        }
    }

    /// Block `index`, verified — and opened, when sealed.
    ///
    /// # Errors
    /// The data file's or the shards' failure; [`crate::Error::Unsealable`] for a block that does not open.
    pub async fn read_block(&self, index: u64) -> Result<Vec<u8>> {
        match self {
            Self::File(reader) => reader.read_block(index).await,
            Self::Erasure(reader) => reader.read_block(index).await,
            Self::Sealed(sealed) => {
                let stored = Box::pin(sealed.stored.read_block(index)).await?;
                sealed.opener.open(index, stored).await
            }
        }
    }
}
