//! The S3 HTTP surface: request parsing, operation dispatch, authentication, and the error envelope.

mod error;
pub mod routes;

pub use error::{Error, Result};
