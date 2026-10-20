//! The `media` domain: objects created in a bucket with media rules, handed to the media workers through the `media`
//! group on `object_events` — at least once, with a deadline, a bounded number of hand-outs and the `media_failed`
//! dead letter — and the renditions those objects need, queued as tasks in `media_images` and `media_videos` for any
//! node's workers to claim; and sweeps, which plan a bucket's existing objects a page at a time through `media_sweeps`.

mod model;
mod repository;
mod service;
mod service_sweeps;
mod service_tasks;
mod sweeps;
mod tasks;
mod tessaridb;

pub use model::{Job, Rendition, Sweep, SweepProgress, Task, TaskQueue};
pub use service::MediaService;
pub use tessaridb::TessariMedia;
