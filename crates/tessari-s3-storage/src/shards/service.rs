//! [`ShardService`]: shard files on this node's drive. A shard is written to exactly the length its request declared
//! — a longer or shorter body leaves nothing behind — and is durable under its final name before `finish` returns.

use tessari_s3_constants::{DATA_BLOCK_SIZE, ERASURE_MAX_WIDTH};

use crate::data::{DataFiles, DataReader, DataWriter};
use crate::{Error, Result};

/// Shard files under the node's data directory.
#[derive(Debug, Clone)]
pub struct ShardService {
    files: Option<DataFiles>,
}

/// A shard being written; [`ShardWrite::finish`] makes it durable, [`ShardWrite::abandon`] removes what was written.
pub struct ShardWrite {
    writer: DataWriter,
    declared: u64,
    received: u64,
}

impl ShardService {
    pub(crate) const fn new(files: Option<DataFiles>) -> Self {
        Self { files }
    }

    fn files(&self) -> Result<&DataFiles> {
        self.files.as_ref().ok_or(Error::NoDataDirectory)
    }

    /// Starts shard `index` of data `id`: `size` bytes in blocks of `block_size`.
    ///
    /// # Errors
    /// [`Error::InvalidShard`] for a block size of zero or past [`DATA_BLOCK_SIZE`] or an index past the widest
    /// code, [`Error::NoDataDirectory`], or the drive's failure (a shard already being written is `AlreadyExists`).
    pub async fn write(
        &self,
        id: [u8; 16],
        index: u16,
        block_size: u32,
        size: u64,
    ) -> Result<ShardWrite> {
        check_index(index)?;
        if block_size == 0 || block_size > DATA_BLOCK_SIZE {
            return Err(Error::InvalidShard {
                reason: "block size of zero or past a data block",
            });
        }
        let writer = self.files()?.create_shard(id, index, block_size).await?;
        Ok(ShardWrite {
            writer,
            declared: size,
            received: 0,
        })
    }

    /// Opens shard `index` of data `id`, which must hold `size` bytes; every block is verified as it is read.
    ///
    /// # Errors
    /// [`Error::InvalidShard`], [`Error::NoDataDirectory`], [`Error::Corrupt`] or the drive's failure (a missing
    /// shard is `NotFound`).
    pub async fn open(&self, id: [u8; 16], index: u16, size: u64) -> Result<DataReader> {
        check_index(index)?;
        self.files()?.open_shard(id, index, size).await
    }

    /// Up to `limit` shards on this node last written more than `age` ago, as (data id, index); none without a data
    /// directory.
    ///
    /// # Errors
    /// The drive's failure while walking.
    pub(crate) async fn older_than(
        &self,
        age: std::time::Duration,
        limit: usize,
    ) -> Result<Vec<([u8; 16], u16)>> {
        match &self.files {
            Some(files) => files.shards_older_than(age, limit).await,
            None => Ok(Vec::new()),
        }
    }

    /// Removes shard `index` of data `id`; a missing shard is not an error.
    ///
    /// # Errors
    /// [`Error::InvalidShard`], [`Error::NoDataDirectory`] or the drive's failure.
    pub async fn remove(&self, id: [u8; 16], index: u16) -> Result<()> {
        check_index(index)?;
        self.files()?.remove_shard(id, index).await
    }
}

fn check_index(index: u16) -> Result<()> {
    if index < u16::from(ERASURE_MAX_WIDTH) {
        Ok(())
    } else {
        Err(Error::InvalidShard {
            reason: "shard index past the widest code",
        })
    }
}

impl ShardWrite {
    /// Appends the next bytes of the shard; bytes past the declared size are refused before any of them is written.
    ///
    /// # Errors
    /// [`Error::ShardLength`] past the declared size, or the drive's failure; either way the caller abandons.
    pub async fn append(&mut self, bytes: &[u8]) -> Result<()> {
        let received = u64::try_from(bytes.len())
            .ok()
            .and_then(|len| self.received.checked_add(len));
        match received {
            Some(received) if received <= self.declared => {
                self.writer.append(bytes).await?;
                self.received = received;
                Ok(())
            }
            _ => Err(Error::ShardLength {
                declared: self.declared,
                received: received.unwrap_or(u64::MAX),
            }),
        }
    }

    /// Makes the shard durable under its final name: the file and its directory are synced before this returns.
    /// A body shorter than declared is abandoned instead.
    ///
    /// # Errors
    /// [`Error::ShardLength`] for a short body, or the drive's failure, a failed sync included; never retried.
    pub async fn finish(self) -> Result<()> {
        if self.received != self.declared {
            let (declared, received) = (self.declared, self.received);
            self.writer.abandon().await?;
            return Err(Error::ShardLength { declared, received });
        }
        self.writer.finish().await.map(|_| ())
    }

    /// Gives the shard up and removes what was written, so the same shard can be written again.
    ///
    /// # Errors
    /// The drive's failure to remove the temporary file.
    pub async fn abandon(self) -> Result<()> {
        self.writer.abandon().await
    }
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
