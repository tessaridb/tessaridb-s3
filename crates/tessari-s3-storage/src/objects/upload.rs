//! An object's bytes on their way into a data file. The data id is queued for reclamation before the file exists,
//! so whatever happens to the process from here on leaves at worst a file the queue names.

use tessari_s3_core::objects::checksum::Digests;

use crate::Result;
use crate::data::DataWriter;

/// A data file being written for an object that is not committed yet.
pub struct Upload {
    id: [u8; 16],
    writer: DataWriter,
}

/// A durable data file, not yet referenced by any object.
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
        Self { id, writer }
    }

    /// The data id.
    #[must_use]
    pub const fn id(&self) -> [u8; 16] {
        self.id
    }

    /// Appends the next bytes of the object.
    ///
    /// # Errors
    /// The drive's failure; the upload is then over.
    pub async fn append(&mut self, bytes: &[u8]) -> Result<()> {
        self.writer.append(bytes).await
    }

    /// Makes the file durable under its final name.
    ///
    /// # Errors
    /// The drive's failure, a failed sync included; it is never retried.
    pub async fn finish(self) -> Result<Uploaded> {
        let finished = self.writer.finish().await?;
        Ok(Uploaded {
            id: self.id,
            size: finished.size,
            digests: finished.digests,
        })
    }
}
