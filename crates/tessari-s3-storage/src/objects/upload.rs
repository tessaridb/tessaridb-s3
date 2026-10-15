//! An object's bytes on their way into a data file, or on a cluster member into k+m shards. The data id is queued
//! for reclamation before any byte is written, so whatever happens to the process from here on leaves at worst
//! bytes the queue names.

use std::sync::Arc;

use tessari_s3_core::objects::checksum::Digests;

use crate::Result;
use crate::data::DataWriter;
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

impl Upload {
    pub(crate) const fn new(id: [u8; 16], writer: DataWriter) -> Self {
        Self {
            id,
            sink: Sink::File(writer),
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
        }
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
    pub async fn append(&mut self, bytes: &[u8]) -> Result<()> {
        match &mut self.sink {
            Sink::File(writer) => writer.append(bytes).await,
            Sink::Erasure { upload, .. } => upload.append(bytes).await,
        }
    }

    /// Makes the bytes durable: the file under its final name, or at least the write quorum of shards — and when
    /// fewer than every shard made it, the id is listed for healing before the upload counts as done.
    ///
    /// # Errors
    /// The drive's failure, a failed sync included (never retried), too few durable shards, or the store's.
    pub async fn finish(self) -> Result<Uploaded> {
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
