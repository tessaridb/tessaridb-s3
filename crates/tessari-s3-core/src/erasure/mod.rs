//! The `erasure` domain: how an object's bytes become k data shards and m parity shards, and how any k of them give
//! the bytes back. Pure computation — the caller moves the bytes and decides where each shard lives.

mod layout;
mod placement;
mod stripe;

pub use layout::{Layout, LayoutError};
pub use placement::{PlacementKey, placement};
pub use stripe::{StripeError, Stripes};
pub use tessari_s3_types::{Code, CodeError};
