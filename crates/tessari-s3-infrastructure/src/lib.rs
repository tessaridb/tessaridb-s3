//! Clients and adapters with no domain rule in them: configuration loading and the TessariDB connection pool.

mod config;
mod error;
pub mod tessaridb;

pub use config::S3Config;
pub use error::{Error, Result};
