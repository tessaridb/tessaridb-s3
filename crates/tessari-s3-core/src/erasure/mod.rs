//! The `erasure` domain: how an object's bytes become k data shards and m parity shards, and how any k of them give
//! the bytes back. Pure computation — the caller moves the bytes and decides where each shard lives.

mod code;
mod stripe;

pub use code::{Code, CodeError};
pub use stripe::{StripeError, Stripes};
