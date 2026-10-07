//! Writing a data file: one bounded block at a time on the blocking pool, then sync, rename and directory sync.

use std::fs::File;
use std::io::{ErrorKind, Write};
use std::path::PathBuf;

use tessari_s3_core::objects::checksum::{Digests, Hashes};

use super::files::{blocking, failed, io, sync_dir};
use super::format::block_hash;
use crate::Result;

/// What a finished write produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Finished {
    /// Object bytes written.
    pub(crate) size: u64,
    /// The object's MD5 and checksums.
    pub(crate) digests: Digests,
}

/// The parts of a write that travel to the blocking pool and back with each block.
struct Sink {
    file: File,
    hashes: Hashes,
    index: u64,
    size: u64,
}

/// A data file being written. Dropping it unfinished leaves the temporary file for removal by its data id.
pub(crate) struct DataWriter {
    sink: Option<Sink>,
    block: Vec<u8>,
    block_size: usize,
    temp: PathBuf,
    path: PathBuf,
    created_dirs: bool,
    hex: String,
}

impl DataWriter {
    pub(super) fn new(
        file: File,
        block_size: usize,
        temp: PathBuf,
        path: PathBuf,
        created_dirs: bool,
        hex: String,
    ) -> Self {
        Self {
            sink: Some(Sink {
                file,
                hashes: Hashes::new(),
                index: 0,
                size: 0,
            }),
            block: Vec::with_capacity(block_size),
            block_size,
            temp,
            path,
            created_dirs,
            hex,
        }
    }

    /// Appends the next bytes of the object; each full block is hashed and written before more is taken, so at
    /// most one block is buffered.
    pub(crate) async fn append(&mut self, mut bytes: &[u8]) -> Result<()> {
        while !bytes.is_empty() {
            let room = self.block_size.saturating_sub(self.block.len());
            let (now, rest) = bytes.split_at(room.min(bytes.len()));
            self.block.extend_from_slice(now);
            bytes = rest;
            if self.block.len() == self.block_size {
                self.flush_block().await?;
            }
        }
        Ok(())
    }

    /// Hashes and writes the buffered block. A failure ends the write: the sink is gone and every later call
    /// fails, because what reached the disk is no longer known.
    async fn flush_block(&mut self) -> Result<()> {
        let mut sink = self
            .sink
            .take()
            .ok_or(failed("write", ErrorKind::BrokenPipe))?;
        let block = std::mem::replace(&mut self.block, Vec::with_capacity(self.block_size));
        let (sink, mut block) = blocking("write", move || {
            sink.hashes.update(&block);
            let hash = block_hash(sink.index, &block);
            sink.file.write_all(&block).map_err(io("write"))?;
            sink.file.write_all(&hash).map_err(io("write"))?;
            let written =
                u64::try_from(block.len()).map_err(|_| failed("write", ErrorKind::InvalidInput))?;
            sink.size = sink
                .size
                .checked_add(written)
                .ok_or(failed("write", ErrorKind::FileTooLarge))?;
            sink.index = sink
                .index
                .checked_add(1)
                .ok_or(failed("write", ErrorKind::FileTooLarge))?;
            Ok((sink, block))
        })
        .await?;
        block.clear();
        self.block = block;
        self.sink = Some(sink);
        Ok(())
    }

    /// Writes the last block, syncs the file, renames it into place and syncs the directory — and, when this
    /// write created the shard directories, their parents too. A failed sync is final: the write fails and is not
    /// retried, because a retried sync can succeed over pages the kernel already dropped.
    pub(crate) async fn finish(mut self) -> Result<Finished> {
        if !self.block.is_empty() {
            self.flush_block().await?;
        }
        let sink = self
            .sink
            .take()
            .ok_or(failed("finish", ErrorKind::BrokenPipe))?;
        let (temp, path, created_dirs, name) = (self.temp, self.path, self.created_dirs, self.hex);
        blocking("finish", move || {
            sink.file.sync_all().map_err(io("sync"))?;
            std::fs::rename(&temp, &path).map_err(io("rename"))?;
            let levels = if created_dirs { 3 } else { 1 };
            let mut dir = path.parent();
            for _ in 0..levels {
                if let Some(current) = dir {
                    sync_dir(current)?;
                    dir = current.parent();
                }
            }
            Ok(Finished {
                size: sink.size,
                digests: sink.hashes.finish(),
            })
        })
        .await
        .inspect_err(|_| tracing::error!(data = %name, "a data file did not reach the disk"))
    }
}
