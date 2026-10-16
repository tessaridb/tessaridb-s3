//! An object's bytes on their way into a data file, or on a cluster member into k+m shards. The data id is queued
//! for reclamation before any byte is written, so whatever happens to the process from here on leaves at worst
//! bytes the queue names. A sealed upload seals the bytes before either sink sees them, and reports the size and
//! digests of what the client sent rather than of what was stored.

use std::sync::Arc;

use tessari_s3_core::objects::checksum::Digests;

use crate::Result;
use crate::data::DataWriter;
use crate::encryption::stream::Sealer;
use crate::erasure::{ErasureUpload, ErasureWrites};

/// Where the bytes go.
enum Sink {
    File(DataWriter),
    Erasure {
        upload: ErasureUpload,
        writes: Arc<ErasureWrites>,
    },
}

/// Data being written for an object that is not committed yet.
pub struct Upload {
    id: [u8; 16],
    sink: Sink,
    sealer: Option<Sealer>,
}

/// Durable data, not yet referenced by any object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Uploaded {
    /// The data id.
    pub id: [u8; 16],
    /// Object bytes written.
    pub size: u64,
    /// The object's MD5 and checksums, computed as the bytes were written.
    pub digests: Digests,
}

impl Sink {
    async fn append(&mut self, bytes: &[u8]) -> Result<()> {
        match self {
            Self::File(writer) => writer.append(bytes).await,
            Self::Erasure { upload, .. } => upload.append(bytes).await,
        }
    }
}

impl Upload {
    pub(crate) const fn new(id: [u8; 16], writer: DataWriter) -> Self {
        Self {
            id,
            sink: Sink::File(writer),
            sealer: None,
        }
    }

    pub(crate) const fn erasure(
        id: [u8; 16],
        upload: ErasureUpload,
        writes: Arc<ErasureWrites>,
    ) -> Self {
        Self {
            id,
            sink: Sink::Erasure { upload, writes },
            sealer: None,
        }
    }

    /// The same upload with every byte sealed by `sealer` before it reaches the sink.
    pub(crate) fn sealing(mut self, sealer: Sealer) -> Self {
        self.sealer = Some(sealer);
        self
    }

    /// The data id.
    #[must_use]
    pub const fn id(&self) -> [u8; 16] {
        self.id
    }

    /// Appends the next bytes of the object.
    ///
    /// # Errors
    /// The drive's failure, or on a cluster member too few shards left for the write quorum; the upload is then over.
    pub async fn append(&mut self, mut bytes: &[u8]) -> Result<()> {
        let Some(sealer) = &mut self.sealer else {
            return self.sink.append(bytes).await;
        };
        while !bytes.is_empty() {
            let (sealed, rest) = sealer.take(bytes).await?;
            if let Some(block) = sealed {
                self.sink.append(&block).await?;
            }
            bytes = rest;
        }
        Ok(())
    }

    /// Makes the bytes durable: the file under its final name, or at least the write quorum of shards — and when
    /// fewer than every shard made it, the id is listed for healing before the upload counts as done.
    ///
    /// # Errors
    /// The drive's failure, a failed sync included (never retried), too few durable shards, or the store's.
    pub async fn finish(mut self) -> Result<Uploaded> {
        let Some(sealer) = self.sealer.take() else {
            return self.finish_sink().await;
        };
        let size = sealer.declared();
        let (last, digests) = sealer.finish().await?;
        if let Some(block) = last {
            self.sink.append(&block).await?;
        }
        let stored = self.finish_sink().await?;
        Ok(Uploaded {
            id: stored.id,
            size,
            digests,
        })
    }

    /// Makes what reached the sink durable, as [`Self::finish`] describes.
    async fn finish_sink(self) -> Result<Uploaded> {
        match self.sink {
            Sink::File(writer) => {
                let finished = writer.finish().await?;
                Ok(Uploaded {
                    id: self.id,
                    size: finished.size,
                    digests: finished.digests,
                })
            }
            Sink::Erasure { upload, writes } => {
                let written = upload.finish().await?;
                if written.durable.iter().any(|durable| !durable) {
                    writes.heal(self.id).await?;
                }
                Ok(Uploaded {
                    id: self.id,
                    size: written.size,
                    digests: written.digests,
                })
            }
        }
    }
}
