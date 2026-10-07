//! Persistence, domain by domain: the metadata records in TessariDB and the data shards on local drives.

pub mod buckets;
mod error;
pub mod multipart;
pub mod objects;

pub use error::{Error, Result};
