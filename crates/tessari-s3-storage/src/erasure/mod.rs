//! The `erasure` domain: an object's bytes written as k data and m parity shards spread over the nodes of the
//! cluster, acknowledged only when the write quorum of shards is durable.

mod senders;
mod upload;

pub use upload::{Destination, ErasurePlan, ErasureUpload, ErasureUploaded};
