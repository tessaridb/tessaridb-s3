//! The `peers` domain: shards stored on other nodes of the cluster, reached over their internal surface by mutual TLS,
//! each end's certificate speaking for its node.

mod service;

pub use service::{PeerAt, RemoteShards, ShardRef};
