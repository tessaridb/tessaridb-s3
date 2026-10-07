//! Clients and adapters with no domain rule in them: configuration loading now, the TessariDB connection pool next.

mod config;
mod error;

pub use config::S3Config;
pub use error::{Error, Result};
