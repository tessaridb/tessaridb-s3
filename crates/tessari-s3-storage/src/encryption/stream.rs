//! An object's bytes sealed as a stream: the plaintext is cut into chunks of one stored block less the tag, so every
//! stored block is exactly one sealed chunk, and block i of the object is block i of whatever holds it — a data file
//! or erasure-coded stripes alike — without the store knowing any key. The stored size follows from the object's
//! size: `size + 16 × ⌈size / chunk⌉`.
//!
//! The last chunk is sealed as the last, so a writer must know the object's size before its first byte and must
//! deliver exactly that many: a short or long body is refused rather than sealed under a wrong last flag. Sealing and
//! opening run on the blocking pool, one chunk at a time; the key is shared with that work read-only.

use std::sync::Arc;

use tessari_s3_core::objects::checksum::{Digests, Hashes};

use super::{BLOCK_TAG_LEN, DataKey, open_block, seal_block};
use crate::{Error, Result};

/// [`BLOCK_TAG_LEN`] as a stored-block length.
const TAG: u32 = 16;
/// [`BLOCK_TAG_LEN`] as a byte count.
const TAG_BYTES: u64 = 16;

const _: () = assert!(BLOCK_TAG_LEN == 16);

/// Plaintext bytes a stored block of `stored` bytes carries; none when the block holds no more than a tag.
pub(crate) fn plain_block(stored: u32) -> Option<u32> {
    stored.checked_sub(TAG).filter(|plain| *plain > 0)
}

/// Stored bytes an object of `size` bytes takes in stored blocks of `stored` bytes.
pub(crate) fn sealed_size(size: u64, stored: u32) -> Option<u64> {
    let chunks = size.div_ceil(u64::from(plain_block(stored)?));
    chunks.checked_mul(TAG_BYTES)?.checked_add(size)
}

/// Runs one seal or open on the blocking pool.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| Error::DataIo {
            op: "seal",
            kind: std::io::ErrorKind::Other,
        })?
}

/// The writing side: plaintext in, sealed blocks out, the plaintext's digests kept along the way.
pub(crate) struct Sealer {
    key: Arc<DataKey>,
    data: [u8; 16],
    chunk: Vec<u8>,
    plain: usize,
    index: u64,
    chunks: u64,
    declared: u64,
    received: u64,
    hashes: Option<Hashes>,
}

impl Sealer {
    /// Seals the `size` bytes of data file `data` under `key` into stored blocks of `stored` bytes.
    ///
    /// # Errors
    /// [`Error::Unsealable`] when a stored block cannot carry a byte.
    pub(crate) fn new(key: DataKey, data: [u8; 16], size: u64, stored: u32) -> Result<Self> {
        let plain = plain_block(stored).ok_or(Error::Unsealable)?;
        let chunk_len = usize::try_from(plain).map_err(|_| Error::Unsealable)?;
        Ok(Self {
            key: Arc::new(key),
            data,
            chunk: Vec::with_capacity(chunk_len),
            plain: chunk_len,
            index: 0,
            chunks: size.div_ceil(u64::from(plain)),
            declared: size,
            received: 0,
            hashes: Some(Hashes::new()),
        })
    }

    /// The object's size, declared before its first byte.
    pub(crate) const fn declared(&self) -> u64 {
        self.declared
    }

    /// Takes the next of `bytes`: the sealed block when they filled a chunk, and what is left of them.
    ///
    /// # Errors
    /// [`Error::SealedLength`] past the declared size; the cipher's or the pool's failure.
    pub(crate) async fn take<'a>(
        &mut self,
        bytes: &'a [u8],
    ) -> Result<(Option<Vec<u8>>, &'a [u8])> {
        let offered = u64::try_from(bytes.len())
            .ok()
            .and_then(|len| self.received.checked_add(len))
            .unwrap_or(u64::MAX);
        if offered > self.declared {
            return Err(Error::SealedLength {
                declared: self.declared,
                received: offered,
            });
        }
        let room = self.plain.saturating_sub(self.chunk.len());
        let (now, rest) = bytes.split_at(room.min(bytes.len()));
        self.chunk.extend_from_slice(now);
        self.received = self
            .received
            .saturating_add(u64::try_from(now.len()).unwrap_or(u64::MAX));
        if self.chunk.len() < self.plain {
            return Ok((None, rest));
        }
        Ok((Some(self.seal_chunk().await?), rest))
    }

    /// Seals what is left; the plaintext's digests.
    ///
    /// # Errors
    /// [`Error::SealedLength`] when fewer bytes than declared arrived; the cipher's or the pool's failure.
    pub(crate) async fn finish(mut self) -> Result<(Option<Vec<u8>>, Digests)> {
        if self.received != self.declared {
            return Err(Error::SealedLength {
                declared: self.declared,
                received: self.received,
            });
        }
        let last = if self.chunk.is_empty() {
            None
        } else {
            Some(self.seal_chunk().await?)
        };
        let hashes = self.hashes.take().ok_or(Error::Unsealable)?;
        Ok((last, hashes.finish()))
    }

    /// Hashes and seals the buffered chunk as the next block, the last when no chunk follows it.
    async fn seal_chunk(&mut self) -> Result<Vec<u8>> {
        let chunk = std::mem::replace(&mut self.chunk, Vec::with_capacity(self.plain));
        let mut hashes = self.hashes.take().ok_or(Error::Unsealable)?;
        let (key, data, index) = (Arc::clone(&self.key), self.data, self.index);
        let last = index.checked_add(1) == Some(self.chunks);
        let (hashes, mut chunk, sealed) = blocking(move || {
            hashes.update(&chunk);
            let sealed = seal_block(&key, data, index, last, &chunk)?;
            Ok((hashes, chunk, sealed))
        })
        .await?;
        chunk.clear();
        self.chunk = chunk;
        self.hashes = Some(hashes);
        self.index = index.checked_add(1).ok_or(Error::Unsealable)?;
        Ok(sealed)
    }
}

/// The reading side: stored block i in, the object's block i out, refused unless it was sealed there under this key.
pub(crate) struct Opener {
    key: Arc<DataKey>,
    data: [u8; 16],
    plain: u32,
    size: u64,
    chunks: u64,
}

impl Opener {
    /// Opens the `size` bytes of data file `data` sealed under `key` into stored blocks of `stored` bytes, `held`
    /// stored bytes in all.
    ///
    /// # Errors
    /// [`Error::Unsealable`] when a stored block cannot carry a byte or `held` is not what `size` lays out.
    pub(crate) fn new(
        key: DataKey,
        data: [u8; 16],
        size: u64,
        stored: u32,
        held: u64,
    ) -> Result<Self> {
        let plain = plain_block(stored).ok_or(Error::Unsealable)?;
        if sealed_size(size, stored) != Some(held) {
            return Err(Error::Unsealable);
        }
        Ok(Self {
            key: Arc::new(key),
            data,
            plain,
            size,
            chunks: size.div_ceil(u64::from(plain)),
        })
    }

    /// The object's size in bytes.
    pub(crate) const fn size(&self) -> u64 {
        self.size
    }

    /// Plaintext bytes per block.
    pub(crate) const fn block_size(&self) -> u32 {
        self.plain
    }

    /// Block `index` of the object, from stored block `index`.
    ///
    /// # Errors
    /// [`Error::Unsealable`] for every reason it does not open; the pool's failure.
    pub(crate) async fn open(&self, index: u64, stored: Vec<u8>) -> Result<Vec<u8>> {
        let (key, data) = (Arc::clone(&self.key), self.data);
        let last = index.checked_add(1) == Some(self.chunks);
        blocking(move || open_block(&key, data, index, last, &stored)).await
    }
}

#[cfg(test)]
#[path = "stream_tests.rs"]
mod tests;
