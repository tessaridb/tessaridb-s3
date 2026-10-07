//! Background work with graceful shutdown: healing, garbage collection, lifecycle.

mod daemon;
mod error;
mod reaper;
mod reclaimer;
mod runner;

pub use daemon::{Daemon, DaemonFuture, Report};
pub use error::{Error, Result};
pub use reaper::UploadReaper;
pub use reclaimer::Reclaimer;
pub use runner::{Runs, run};
