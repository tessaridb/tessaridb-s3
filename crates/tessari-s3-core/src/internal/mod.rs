//! The `internal` domain: how one node of the cluster proves to another that a shard request is the cluster's own.

mod signature;

pub use signature::{InternalAuthError, InternalRequest, sign, verify};
