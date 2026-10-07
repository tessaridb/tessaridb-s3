//! The metadata store: TessariDB over its wire protocol.

mod error;
mod pool;
mod settings;

pub use error::{MetaError, MetaResult};
pub use pool::MetaPool;
pub use settings::{MetaSettings, is_safe_name};
pub use tessaridb_client::{Answer, Number, RefusalClass, Value};
