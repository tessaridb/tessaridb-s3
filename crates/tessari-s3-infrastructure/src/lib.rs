//! Clients and adapters with no domain rule in them: configuration loading, the TessariDB connection pool and the
//! HTTP client nodes call each other with.

mod bootstrap_config;
mod cluster_config;
mod config;
mod console_config;
pub mod credential;
mod error;
mod iam_config;
mod kafka;
mod keys_config;
mod live_settings;
pub mod media_image;
pub mod media_video;
mod meta_config;
mod notify_config;
mod observability_config;
pub mod oidc;
mod oidc_client_key;
mod oidc_config;
mod oidc_http;
mod oidc_logout;
mod oidc_verify;
mod oidc_web_identity;
pub mod peer;
mod replica_client;
pub mod settings_keys;
pub mod settings_source;
pub mod tessaridb;
pub mod tls;
mod webhook;

pub use bootstrap_config::Bootstrap;
pub use config::{ClusterSettings, S3Config};
pub use console_config::{ClientAuth, ConsoleSettings, KeySignIn, OidcSettings};
pub use error::{Error, Result};
pub use kafka::KafkaProducer;
pub use live_settings::{Live, LiveSettings};
pub use notify_config::{NotifyEndpoint, NotifyTarget};
pub use oidc_client_key::ClientKey;
pub use replica_client::ReplicaClient;
pub use tls::InternalTls;
pub use webhook::WebhookClient;
