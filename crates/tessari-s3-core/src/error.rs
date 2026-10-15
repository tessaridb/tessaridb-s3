//! Failures of `tessari-s3-core`.

/// Everything that can go wrong in this crate. It has no variants yet: none of its operations exist.
#[derive(Debug, thiserror::Error)]
pub enum Error {}

/// The result of an operation in this crate.
pub type Result<T> = std::result::Result<T, Error>;
