//! The `peers` domain: shards stored on other nodes of the cluster, reached over their internal surface with every
//! request signed by the cluster secret.

mod service;

pub use service::{RemoteShards, ShardRef};
