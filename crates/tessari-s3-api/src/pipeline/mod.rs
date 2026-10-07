//! The request pipeline from the first byte to the operation: addressing, query, dispatch, authentication, the
//! response envelope and load shedding.

mod address;
mod authenticate;
mod handler;
mod overload;
mod query;
mod response;

pub use handler::handle;
pub use overload::refused;
