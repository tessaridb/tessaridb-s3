//! Persistence, domain by domain: the metadata records in TessariDB and the data shards on local drives.

mod answers;
pub mod buckets;
pub mod cluster;
pub mod data;
mod error;
pub mod multipart;
pub mod objects;
pub mod peers;
mod schema;
pub mod shards;
mod storage;

pub use error::{Error, Result};
pub use storage::Storage;
