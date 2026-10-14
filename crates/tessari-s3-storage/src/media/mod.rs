//! The `media` domain: objects created in a bucket with media rules, handed to the media workers through the `media`
//! group on `object_events` — at least once, with a deadline, a bounded number of hand-outs and the `media_failed`
//! dead letter.

mod model;
mod repository;
mod service;
mod tessaridb;

pub use model::Job;
pub use service::MediaService;
pub use tessaridb::TessariMedia;
