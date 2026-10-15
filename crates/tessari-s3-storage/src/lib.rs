//! Persistence, domain by domain: the metadata records in TessariDB and the data shards on local drives.

pub mod actions;
mod answers;
mod backoff;
pub mod buckets;
pub mod cluster;
pub mod data;
pub mod erasure;
mod error;
pub mod identities;
pub mod lifecycle;
pub mod media;
pub mod members;
pub mod multipart;
pub mod notifications;
pub mod objects;
mod params;
pub mod peers;
pub mod regions;
pub mod replication;
mod schema;
mod schema_queues;
pub mod shards;
pub mod spaces;
mod storage;
mod topic_groups;
pub mod usage;
pub mod users;

pub use error::{Error, Result};
pub use storage::Storage;
