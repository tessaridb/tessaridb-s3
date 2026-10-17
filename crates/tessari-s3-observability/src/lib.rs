//! Tracing and metrics set-up shared by every process.

mod error;
mod metrics;
mod metrics_labels;
mod subscriber;

pub use error::{Error, Result};
pub use metrics::Metrics;
pub use metrics_labels::{AuthRefusal, Served, size_class};
pub use subscriber::init;
