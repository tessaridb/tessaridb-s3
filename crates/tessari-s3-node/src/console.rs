//! The operator console a node serves when `TESSARIDB_S3_CONSOLE_LISTEN` is set (ADR-0006).

use tessari_s3_api::{ApiState, ConsoleState, serve_console};
use tessari_s3_infrastructure::S3Config;
use tessari_s3_infrastructure::tls::console_server_config;
use tessari_s3_storage::Storage;
use tokio::sync::watch;

/// The console, bound before anything is served and stopped with the other surfaces; `None` when not configured.
pub(crate) async fn surface(
    config: &S3Config,
    storage: &Storage,
    mut stop: watch::Receiver<bool>,
) -> anyhow::Result<Option<impl Future<Output = std::io::Result<()>> + use<>>> {
    let (Some(console), Some(state)) = (
        &config.console,
        ConsoleState::new(config, ApiState::system_clock(), storage.clone()),
    ) else {
        return Ok(None);
    };
    let tls = console.tls.as_ref().map(console_server_config).transpose()?;
    let listener = tokio::net::TcpListener::bind(console.listen).await?;
    if tls.is_some() {
        tracing::info!(console_listen = %console.listen, "console listening over TLS");
    } else {
        tracing::warn!(
            console_listen = %console.listen,
            "console listening in the clear: the root secret crosses the network unencrypted at sign-in; set \
             TESSARIDB_S3_CONSOLE_TLS_CERT and _KEY, or keep the address private"
        );
    }
    Ok(Some(serve_console(listener, state, tls, async move {
        let _closed = stop.changed().await;
    })))
}
