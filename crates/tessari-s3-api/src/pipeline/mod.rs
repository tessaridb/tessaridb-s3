//! The request pipeline from the first byte to the operation: addressing, query, dispatch, authentication, the
//! response envelope and load shedding.

pub(crate) mod address;
mod authenticate;
pub(crate) mod body;
pub(crate) mod call;
mod handler;
mod overload;
mod query;
pub(crate) mod response;

pub use handler::handle;
pub use overload::refused;
