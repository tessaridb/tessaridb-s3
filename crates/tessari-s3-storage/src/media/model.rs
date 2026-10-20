//! An object a media worker makes renditions of.

use tessari_s3_core::media::{Output, Rule};

use crate::notifications::Event;

/// A created object in a bucket whose rules may take it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    /// The change, as the store's event recorded it.
    pub event: Event,
    /// The bucket's media rules as they stand now; which apply depends on the object's content type.
    pub rules: Vec<Rule>,
}

/// One rendition to make: an output of one stored version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendition {
    /// The bucket holding the original.
    pub bucket: String,
    /// The original's key.
    pub key: String,
    /// The original's version, when the bucket keeps versions.
    pub version_id: Option<String>,
    /// The original's data id: the version the rendition is made from, and the task's identity with the output.
    pub source: [u8; 16],
    /// What to make.
    pub output: Output,
}

/// The queue a rendition waits in, by the encoder it needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskQueue {
    /// `media_images`: images, made on any node.
    Images,
    /// `media_videos`: videos and posters, made on nodes with a video encoder.
    Videos,
}

impl TaskQueue {
    /// The queue `output` waits in.
    #[must_use]
    pub const fn of(output: Output) -> Self {
        match output {
            Output::Image { .. } => Self::Images,
            Output::Video { .. } | Output::Poster { .. } => Self::Videos,
        }
    }

    /// The queue's table — this crate's own schema name, never a caller's.
    pub(crate) const fn table(self) -> &'static str {
        match self {
            Self::Images => "media_images",
            Self::Videos => "media_videos",
        }
    }
}

/// A rendition waiting in, or held from, its queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    /// The task's identity in its queue.
    pub name: String,
    /// The queue it was taken from.
    pub queue: TaskQueue,
    /// The consumer holding it, when one does.
    pub consumer: Option<String>,
    /// What to make.
    pub rendition: Rendition,
    /// How many times it has been handed out, this time included.
    pub attempts: i64,
    /// Why the last attempt failed, if one did.
    pub error: Option<String>,
}

/// A bucket's sweep of its existing objects, as a pass holds it: the keys up to `after` are planned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sweep {
    /// The bucket swept.
    pub bucket: String,
    /// The last key planned; empty before the first page.
    pub after: String,
    /// Objects listed so far.
    pub listed: u64,
    /// Renditions queued so far.
    pub queued: u64,
    /// When it was started, which tells this sweep from one started again since.
    pub started: (i64, u32),
}

/// Where a bucket's sweep stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SweepProgress {
    /// No sweep was started.
    None,
    /// Under way: so many objects listed and renditions queued.
    Running {
        /// Objects listed so far.
        listed: u64,
        /// Renditions queued so far.
        queued: u64,
    },
    /// The last sweep finished.
    Done {
        /// Objects listed.
        listed: u64,
        /// Renditions queued.
        queued: u64,
    },
}
