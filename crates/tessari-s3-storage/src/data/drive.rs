//! How full this node's data drive is: the filesystem holding the data directory, as the operating system reports it.

use std::sync::atomic::Ordering;

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

impl DataFiles {
    /// Starts no new data file while the drive has fewer than `bytes` available to this process; 0 turns the floor
    /// off. Every clone shares it.
    pub(crate) fn set_floor(&self, bytes: u64) {
        self.floor.store(bytes, Ordering::Relaxed);
    }

    /// [`Error::DriveFull`] while the drive is below the floor. One `statvfs` per new file, off the async workers.
    pub(super) async fn check_floor(&self) -> Result<()> {
        let floor = self.floor.load(Ordering::Relaxed);
        if floor == 0 {
            return Ok(());
        }
        let available = self.drive().await?.available;
        if available < floor {
            return Err(Error::DriveFull { available, floor });
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "drive_tests.rs"]
mod tests;
