//! An object a media worker makes renditions of.

use tessari_s3_core::media::Rule;

use crate::notifications::Event;

/// A created object in a bucket whose rules may take it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    /// The change, as the store's event recorded it.
    pub event: Event,
    /// The bucket's media rules as they stand now; which apply depends on the object's content type.
    pub rules: Vec<Rule>,
}
