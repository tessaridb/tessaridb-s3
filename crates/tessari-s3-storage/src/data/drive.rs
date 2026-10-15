//! How full this node's data drive is: the filesystem holding the data directory, as the operating system reports it.

use super::files::{DataFiles, io};
use crate::{Error, Result};

/// The data drive's space, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DriveSpace {
    /// Its size.
    pub capacity: u64,
    /// Space not in use, including what only the superuser may write.
    pub free: u64,
    /// Space this process may still write.
    pub available: u64,
}

impl DriveSpace {
    /// The form one node sends another: three decimal byte counts, `capacity free available`.
    #[must_use]
    pub fn to_wire(self) -> String {
        format!("{} {} {}", self.capacity, self.free, self.available)
    }

    /// Reads [`DriveSpace::to_wire`]'s form; `None` for anything else.
    #[must_use]
    pub fn from_wire(text: &str) -> Option<Self> {
        let mut parts = text.split(' ').map(str::parse::<u64>);
        let (Some(Ok(capacity)), Some(Ok(free)), Some(Ok(available)), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return None;
        };
        Some(Self {
            capacity,
            free,
            available,
        })
    }
}

impl DataFiles {
    /// The space of the filesystem holding the data directory — before the first write creates it, the filesystem it
    /// will be created on. The call blocks on the drive, so it runs off the async workers.
    pub(crate) async fn drive(&self) -> Result<DriveSpace> {
        let root = self.data_root();
        tokio::task::spawn_blocking(move || {
            let existing = root
                .ancestors()
                .find(|path| path.exists())
                .unwrap_or(root.as_path());
            let stats = rustix::fs::statvfs(existing)
                .map_err(|errno| io("statvfs")(std::io::Error::from(errno)))?;
            let bytes = |blocks: u64| {
                blocks.checked_mul(stats.f_frsize).ok_or(Error::Malformed {
                    record: "drive",
                    reason: "size overflows",
                })
            };
            Ok(DriveSpace {
                capacity: bytes(stats.f_blocks)?,
                free: bytes(stats.f_bfree)?,
                available: bytes(stats.f_bavail)?,
            })
        })
        .await
        .map_err(|_| Error::DataIo {
            op: "statvfs",
            kind: std::io::ErrorKind::Other,
        })?
    }
}

#[cfg(test)]
#[path = "drive_tests.rs"]
mod tests;
