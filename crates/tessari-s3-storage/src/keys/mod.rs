//! The `keys` domain: the key-encryption keys object data keys are wrapped under, held in an engine vault with its
//! own passphrase and brought into this node's memory only by an operator's unseal.

mod entity;
mod model;
mod repository;
mod service;
mod tessaridb;

pub(crate) use model::SERVER_KEY_PREFIX;
pub use model::{KeyRing, KeyState, Passphrase};
pub use service::KeyService;
pub use tessaridb::TessariKeys;
