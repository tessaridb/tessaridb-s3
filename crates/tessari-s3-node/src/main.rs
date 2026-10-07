//! The `tessaridb-s3` process: the composition root, and nothing else.
//!
//! Order: tracing, then configuration, infrastructure, storage and the servers (none exist yet), then a
//! shutdown that waits for SIGINT or SIGTERM.

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    tessari_s3_observability::init();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "tessaridb-s3 starting");
    shutdown_signal().await?;
    tracing::info!("tessaridb-s3 stopping");
    Ok(())
}

/// Resolves when the process is asked to stop.
async fn shutdown_signal() -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            biased;
            result = tokio::signal::ctrl_c() => result?,
            _ = terminate.recv() => {}
        }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await?;
    Ok(())
}
