//! Data files on the node's drive. A write goes to a temporary name in the final directory, is synced, renamed and
//! the directory synced, so a crash leaves either nothing under the final name or the whole file. Every block is
//! hashed as it is written and verified as it is read; hashing and file I/O run on the blocking pool, one bounded
//! block at a time, so the runtime's workers never wait on a disk or a hash.

use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::format::{
    HASH_LEN, HEADER_LEN, Piece, block_hash, block_span, decode_header, encode_header, file_len,
};
use super::writer::DataWriter;
use crate::{Error, Result};

/// The directory under the data root that holds every data file.
const DATA_SUBDIR: &str = "s3data";

/// Where data files live and the block size new files are written with.
#[derive(Debug, Clone)]
pub(crate) struct DataFiles {
    root: PathBuf,
    block_size: u32,
}

/// An open data file whose header and length were checked. The handle is shared read-only with the blocking pool:
/// positioned reads take `&File`, so there is no cursor to contend for.
pub struct DataReader {
    file: Arc<File>,
    size: u64,
    block_size: u32,
    hex: String,
}

pub(crate) fn hex(id: [u8; 16]) -> String {
    id.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(super) fn io(op: &'static str) -> impl Fn(std::io::Error) -> Error {
    move |error| {
        tracing::error!(op, error = %error, "data file operation failed");
        Error::DataIo {
            op,
            kind: error.kind(),
        }
    }
}

pub(super) const fn failed(op: &'static str, kind: ErrorKind) -> Error {
    Error::DataIo { op, kind }
}

/// Runs `work` on the blocking pool; a panic there is reported as a failed operation rather than unwinding here.
/// Every closure handed here does one bounded step (a header, one block, one sync), so none needs a cancel flag.
pub(super) async fn blocking<T: Send + 'static>(
    op: &'static str,
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| failed(op, ErrorKind::Other))?
}

/// Syncs a directory so the entries created or renamed in it survive a power loss.
pub(super) fn sync_dir(dir: &Path) -> Result<()> {
    File::open(dir)
        .and_then(|handle| handle.sync_all())
        .map_err(io("sync directory"))
}

impl DataFiles {
    /// Files under `root`, written in blocks of `block_size` bytes.
    pub(crate) const fn new(root: PathBuf, block_size: u32) -> Self {
        Self { root, block_size }
    }

    /// The final path of data `id`: two directory levels from its first two bytes, so no directory holds more than
    /// a 65 536th of the files. Only the server's own uuid names a path; nothing a client sent reaches it.
    pub(crate) fn path(&self, id: [u8; 16]) -> PathBuf {
        let name = hex(id);
        self.root
            .join(DATA_SUBDIR)
            .join(&name[0..2])
            .join(&name[2..4])
            .join(name)
    }

    /// The final and temporary paths of `piece` of data `id`. A shard sits beside the whole-object name with its
    /// index as a suffix, so the two never share a file.
    pub(super) fn piece_paths(&self, id: [u8; 16], piece: Piece) -> (PathBuf, PathBuf) {
        let path = self.path(id);
        match piece {
            Piece::Whole => {
                let temp = path.with_extension("tmp");
                (path, temp)
            }
            Piece::Shard(index) => {
                let mut name = path.into_os_string();
                name.push(format!(".s{index}"));
                let shard = PathBuf::from(name);
                let mut temp = shard.clone().into_os_string();
                temp.push(".tmp");
                (shard, PathBuf::from(temp))
            }
        }
    }

    /// Starts writing data `id`: its directory exists and its temporary file holds the header.
    pub(crate) async fn create(&self, id: [u8; 16]) -> Result<DataWriter> {
        self.create_piece(id, Piece::Whole, self.block_size).await
    }

    /// Starts writing `piece` of data `id` in blocks of `block_size` bytes.
    pub(super) async fn create_piece(
        &self,
        id: [u8; 16],
        piece: Piece,
        block_size: u32,
    ) -> Result<DataWriter> {
        let (path, temp) = self.piece_paths(id, piece);
        let header = encode_header(block_size, id, piece);
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        let open_temp = temp.clone();
        let (file, created_dirs) = blocking("create", move || {
            let created_dirs = !dir.exists();
            std::fs::create_dir_all(&dir).map_err(io("create directory"))?;
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&open_temp)
                .map_err(io("create"))?;
            file.write_all(&header).map_err(io("write"))?;
            Ok((file, created_dirs))
        })
        .await?;
        let block_size =
            usize::try_from(block_size).map_err(|_| failed("create", ErrorKind::InvalidInput))?;
        Ok(DataWriter::new(
            file,
            block_size,
            temp,
            path,
            created_dirs,
            hex(id),
        ))
    }

    /// Opens data `id`, which must hold an object of `size` bytes: the header must be ours and name this id, and
    /// the file's length must be exactly what that size lays out.
    pub(crate) async fn open(&self, id: [u8; 16], size: u64) -> Result<DataReader> {
        self.open_piece(id, Piece::Whole, size).await
    }

    /// Opens `piece` of data `id`, which must hold `size` bytes.
    pub(super) async fn open_piece(
        &self,
        id: [u8; 16],
        piece: Piece,
        size: u64,
    ) -> Result<DataReader> {
        let (path, _) = self.piece_paths(id, piece);
        let name = hex(id);
        let corrupt_name = name.clone();
        let corrupt = move |reason| Error::Corrupt {
            id: corrupt_name.clone(),
            reason,
        };
        let (file, block_size) = blocking("open", move || {
            let file = File::open(&path).map_err(io("open"))?;
            let mut header = [0; HEADER_LEN];
            file.read_exact_at(&mut header, 0)
                .map_err(|_| corrupt("header"))?;
            let block_size = decode_header(&header, id, piece).map_err(|_| corrupt("header"))?;
            let length = file.metadata().map_err(io("stat"))?.len();
            if file_len(size, block_size) != Some(length) {
                return Err(corrupt("length"));
            }
            Ok((file, block_size))
        })
        .await?;
        Ok(DataReader {
            file: Arc::new(file),
            size,
            block_size,
            hex: name,
        })
    }

    /// Removes data `id` under its final and its temporary name; a missing file is not an error.
    pub(crate) async fn remove(&self, id: [u8; 16]) -> Result<()> {
        self.remove_piece(id, Piece::Whole).await
    }

    /// Removes `piece` of data `id` under its final and its temporary name; a missing file is not an error.
    pub(super) async fn remove_piece(&self, id: [u8; 16], piece: Piece) -> Result<()> {
        let (path, temp) = self.piece_paths(id, piece);
        blocking("remove", move || {
            for name in [temp, path] {
                match std::fs::remove_file(&name) {
                    Err(error) if error.kind() != ErrorKind::NotFound => {
                        return Err(io("remove")(error));
                    }
                    _ => {}
                }
            }
            Ok(())
        })
        .await
    }
}

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

#[cfg(test)]
#[path = "files_tests.rs"]
mod tests;
