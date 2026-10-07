//! Operation dispatch: a decision table over the 116 operations of the S3 model, derived from the model itself.
//! A request resolves to exactly one operation or is refused; nothing unrecognised reaches a handler.

mod catalog;
mod error;
mod spec;
mod support;
mod table;

pub use catalog::{CATALOG, Operation};
pub use error::DispatchError;
pub use spec::{Method, OperationSpec, Target};
pub use support::is_implemented;
pub use table::{DispatchRequest, dispatch};
