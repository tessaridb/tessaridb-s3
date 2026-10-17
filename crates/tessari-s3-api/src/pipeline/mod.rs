//! The request pipeline from the first byte to the operation: addressing, query, dispatch, authentication, the
//! response envelope and load shedding.

pub(crate) mod address;
mod authenticate;
pub(crate) mod authorize;
pub(crate) mod body;
pub(crate) mod call;
mod fence;
mod handler;
mod overload;
mod post_form;
mod query;
pub(crate) mod response;
pub(crate) mod sts;
mod trail;

pub use handler::handle;
pub use overload::refused;
