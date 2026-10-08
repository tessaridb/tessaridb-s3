//! Persistence, domain by domain: the metadata records in TessariDB and the data shards on local drives.

mod answers;
pub mod buckets;
pub mod cluster;
pub mod data;
mod error;
pub mod multipart;
pub mod objects;
mod schema;
mod storage;

pub use error::{Error, Result};
pub use storage::Storage;
