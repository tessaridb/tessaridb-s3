//! A load generator for the TessariDB S3 API: signed requests per size class and concurrency step, latency
//! percentiles measured from the client, every GET checked against what was PUT, and a report that carries the
//! build and host it was measured on.

mod body;
mod client;
mod config;
mod error;
mod measure;
mod ops;
mod report;
mod run;
mod summarise;
mod workload;

pub use body::SizeClass;
pub use config::BenchConfig;
pub use error::{Error, Result};
pub use report::{Phase, Report};
pub use run::{read_reports, run};
pub use summarise::summarise;
pub use workload::Plan;
