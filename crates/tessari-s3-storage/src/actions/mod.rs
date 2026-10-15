//! The `actions` domain: the console's record of what an operator did, to what, and why — a TessariDB topic, so the
//! store itself refuses to change or remove an entry.

mod entity;
mod model;
mod repository;
mod service;
mod tessaridb;

pub use model::{Action, NewAction};
pub use service::ActionService;
pub use tessaridb::TessariActions;
