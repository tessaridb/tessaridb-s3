//! The `data` domain: object bytes in data files on the node's drive, every block verified on read.

mod drive;
mod files;
mod format;
mod shards;
mod writer;

pub use drive::DriveSpace;
pub use files::DataReader;
pub(crate) use files::{DataFiles, hex};
pub(crate) use writer::DataWriter;
