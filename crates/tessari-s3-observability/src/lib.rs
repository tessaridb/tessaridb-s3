//! Tracing and metrics set-up shared by every process.

mod error;
mod subscriber;

pub use error::{Error, Result};
pub use subscriber::init;
