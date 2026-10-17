//! The metadata store: TessariDB over its wire protocol.

mod error;
mod pool;
mod settings;

pub use error::{MetaError, MetaResult};
pub use pool::{MetaPool, VaultLink};
pub use settings::{MetaSettings, Replication, is_region_name, is_safe_name, regional_database};
pub use tessaridb_client::{
    Answer, Number, RecordId, RecordRef, RefusalClass, SealState, Value, VaultStatus,
};
