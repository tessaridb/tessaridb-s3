//! Shards of erasure-coded objects on this node's drive: each a data file of its own (format 2) whose header names its
//! index, so a shard can never be read as another shard of the same data or as a whole object.

use super::files::DataFiles;
use super::format::Piece;
use super::{DataReader, DataWriter};
use crate::Result;

impl DataFiles {
    /// The final path of shard `index` of data `id`.
    #[cfg(test)]
    pub(crate) fn shard_path(&self, id: [u8; 16], index: u16) -> std::path::PathBuf {
        self.piece_paths(id, Piece::Shard(index)).0
    }

    /// Starts writing shard `index` of data `id` in blocks of `block_size` bytes (the erasure block length).
    pub(crate) async fn create_shard(
        &self,
        id: [u8; 16],
        index: u16,
        block_size: u32,
    ) -> Result<DataWriter> {
        self.create_piece(id, Piece::Shard(index), block_size).await
    }

    /// Opens shard `index` of data `id`, which must hold `size` bytes.
    pub(crate) async fn open_shard(
        &self,
        id: [u8; 16],
        index: u16,
        size: u64,
    ) -> Result<DataReader> {
        self.open_piece(id, Piece::Shard(index), size).await
    }

    /// Removes shard `index` of data `id`; a missing shard is not an error.
    pub(crate) async fn remove_shard(&self, id: [u8; 16], index: u16) -> Result<()> {
        self.remove_piece(id, Piece::Shard(index)).await
    }
}
