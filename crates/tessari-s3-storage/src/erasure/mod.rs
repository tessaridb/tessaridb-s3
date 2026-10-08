//! The `erasure` domain: an object's bytes written as k data and m parity shards spread over the nodes of the
//! cluster, acknowledged only when the write quorum of shards is durable.

mod heal;
mod read;
mod repository;
mod senders;
mod tessaridb;
mod upload;
mod writes;

pub use read::ErasureReader;
pub use upload::{Destination, ErasurePlan, ErasureUpload, ErasureUploaded};
pub(crate) use writes::ErasureWrites;
