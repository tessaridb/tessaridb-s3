//! The `shards` domain: this node's share of erasure-coded objects — one data file per shard, written, read and
//! removed on behalf of the node coordinating the object.

mod service;

pub use service::{ShardService, ShardWrite};
