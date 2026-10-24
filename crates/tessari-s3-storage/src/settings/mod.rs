//! The `settings` domain: the settings every node of a region runs on, kept in the metadata store — plain settings as
//! records, secrets in a vault the keys passphrase opens — written there from the environment the first time a node
//! starts with them.

mod model;
mod repository;
mod service;
mod tessaridb;

pub use service::SettingsService;
pub use tessaridb::TessariSettings;
