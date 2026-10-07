//! The S3 object model as rules and use cases, with no I/O: what a bucket, an object, a version and a multipart upload
//! may be, and what each operation does to them.

pub mod auth;
pub mod buckets;
mod error;
pub mod multipart;
pub mod objects;

pub use error::{Error, Result};
