//! The `data` domain: object bytes in data files on the node's drive, every block verified on read.

mod files;
mod format;
mod writer;

pub(crate) use files::DataFiles;
pub use files::DataReader;
pub(crate) use writer::DataWriter;
