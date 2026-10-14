//! Clients and adapters with no domain rule in them: configuration loading, the TessariDB connection pool and the
//! HTTP client nodes call each other with.

mod cluster_config;
mod config;
mod console_config;
mod error;
mod iam_config;
mod kafka;
pub mod media_image;
pub mod media_video;
mod meta_config;
mod notify_config;
pub mod oidc;
mod oidc_config;
mod oidc_http;
mod oidc_logout;
mod oidc_verify;
pub mod peer;
pub mod tessaridb;
pub mod tls;
mod webhook;

pub use config::{ClusterSettings, S3Config};
pub use console_config::{ConsoleSettings, OidcSettings};
pub use error::{Error, Result};
pub use kafka::KafkaProducer;
pub use notify_config::{NotifyEndpoint, NotifyTarget};
pub use tls::InternalTls;
pub use webhook::WebhookClient;
