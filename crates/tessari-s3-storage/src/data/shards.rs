//! Shards of erasure-coded objects on this node's drive: each a data file of its own (format 2) whose header names its
//! index, so a shard can never be read as another shard of the same data or as a whole object.

use std::path::Path;
use std::time::{Duration, SystemTime};

use super::files::{DataFiles, blocking, io};
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

impl DataFiles {
    /// Up to `limit` shards on this drive last written more than `age` ago, as (data id, index). Temporary files are
    /// never listed: a shard being written has no final name yet.
    pub(crate) async fn shards_older_than(
        &self,
        age: Duration,
        limit: usize,
    ) -> Result<Vec<([u8; 16], u16)>> {
        let root = self.data_root();
        blocking("list", move || {
            let cutoff = SystemTime::now()
                .checked_sub(age)
                .unwrap_or(SystemTime::UNIX_EPOCH);
            let mut found = Vec::new();
            walk(&root, cutoff, limit, &mut found)?;
            Ok(found)
        })
        .await
    }
}

fn walk(
    dir: &Path,
    cutoff: SystemTime,
    limit: usize,
    found: &mut Vec<([u8; 16], u16)>,
) -> Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(io("list")(error)),
    };
    for entry in entries {
        if found.len() >= limit {
            return Ok(());
        }
        let entry = entry.map_err(io("list"))?;
        let path = entry.path();
        if path.is_dir() {
            walk(&path, cutoff, limit, found)?;
            continue;
        }
        let Some(shard) = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(shard_name)
        else {
            continue;
        };
        let modified = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .map_err(io("list"))?;
        if modified <= cutoff {
            found.push(shard);
        }
    }
    Ok(())
}

/// The data id and index a final shard name `<32 hex digits>.s<index>` spells; anything else is `None`.
fn shard_name(name: &str) -> Option<([u8; 16], u16)> {
    let (hex, index) = name.split_once(".s")?;
    let index = index.parse::<u16>().ok()?;
    if hex.len() != 32 {
        return None;
    }
    let mut id = [0_u8; 16];
    for (byte, pair) in id.iter_mut().zip(hex.as_bytes().chunks(2)) {
        *byte = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some((id, index))
}

#[cfg(test)]
mod tests {
    use super::shard_name;

    #[test]
    fn only_a_final_shard_name_is_read_as_a_shard() {
        let hex = "00112233445566778899aabbccddeeff";
        let id = [
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
            0xee, 0xff,
        ];
        assert_eq!(shard_name(&format!("{hex}.s5")), Some((id, 5)));
        for name in [
            format!("{hex}.s5.tmp"),
            format!("{hex}.tmp"),
            hex.to_owned(),
            format!("{hex}.sx"),
            format!("{}.s1", &hex[1..]),
            format!("{}zz.s1", &hex[2..]),
        ] {
            assert_eq!(shard_name(&name), None, "{name}");
        }
    }
}
