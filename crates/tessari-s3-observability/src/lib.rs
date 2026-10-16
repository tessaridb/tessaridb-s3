//! Tracing and metrics set-up shared by every process.

mod error;
mod metrics;
mod subscriber;

pub use error::{Error, Result};
pub use metrics::{Metrics, Served, size_class};
pub use subscriber::init;
