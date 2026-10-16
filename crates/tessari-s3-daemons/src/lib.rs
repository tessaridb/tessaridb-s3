//! Background work with graceful shutdown: healing, garbage collection, lifecycle, usage, notification delivery,
//! media renditions.

mod daemon;
mod error;
mod healer;
mod lifecycle;
mod media_images;
mod media_planner;
mod media_videos;
mod media_work;
mod mover;
mod notify;
mod reaper;
mod reclaimer;
mod replicator;
mod rewrapper;
mod runner;
mod usage;

pub use daemon::{Daemon, DaemonFuture, Report};
pub use error::{Error, Result};
pub use healer::Healer;
pub use lifecycle::LifecycleWorker;
pub use media_images::ImageWorker;
pub use media_planner::MediaPlanner;
pub use media_videos::VideoWorker;
pub use mover::Mover;
pub use notify::{Destination, NotifyWorker};
pub use reaper::UploadReaper;
pub use reclaimer::Reclaimer;
pub use replicator::Replicator;
pub use rewrapper::Rewrapper;
pub use runner::{Runs, run};
pub use usage::UsageScanner;
