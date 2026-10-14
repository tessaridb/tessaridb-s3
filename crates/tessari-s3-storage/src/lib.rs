//! Persistence, domain by domain: the metadata records in TessariDB and the data shards on local drives.

pub mod actions;
mod answers;
pub mod buckets;
pub mod cluster;
pub mod data;
pub mod erasure;
mod error;
pub mod lifecycle;
pub mod multipart;
pub mod objects;
mod params;
pub mod peers;
mod schema;
mod schema_queues;
pub mod shards;
pub mod spaces;
mod storage;
pub mod usage;
pub mod users;

pub use error::{Error, Result};
pub use storage::Storage;
