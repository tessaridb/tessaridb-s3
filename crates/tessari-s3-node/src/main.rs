//! The `tessaridb-s3` process: the composition root, and nothing else.
//!
//! Order: tracing, configuration, the metadata pool and schema, the API state and router, the listener, then serving
//! until SIGINT or SIGTERM,
//! after which in-flight requests get the configured grace period before the process exits.

use std::time::Duration;

use tessari_s3_api::{ApiState, router};
use tessari_s3_infrastructure::S3Config;
use tessari_s3_infrastructure::tessaridb::MetaPool;
use tessari_s3_storage::Storage;
use tokio::sync::watch;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    tessari_s3_observability::init();
    let config = S3Config::from_env()?;
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        listen = %config.listen,
        region = %config.region,
        domains = ?config.domains,
        max_inflight = config.max_inflight,
        "tessaridb-s3 starting"
    );
    let storage = Storage::new(MetaPool::new(config.meta.clone())?, config.data_dir.clone());
    // The schema is applied before the listener opens: a node whose metadata store cannot be reached does not
    // start, rather than answering every request 503.
    storage.prepare().await?;
    tracing::info!(meta = %config.meta.address, namespace = %config.meta.namespace, "metadata schema ready");
    let app = router(ApiState::new(&config, ApiState::system_clock(), storage));
    let listener = tokio::net::TcpListener::bind(config.listen).await?;
    let (stop_tx, mut stop_rx) = watch::channel(false);
    let server = axum::serve(listener, app).with_graceful_shutdown(async move {
        // A closed sender also means stop: the select below has returned.
        let _closed = stop_rx.changed().await;
    });
    let mut server = std::pin::pin!(server.into_future());
    tokio::select! {
        biased;
        signal = shutdown_signal() => {
            signal?;
            tracing::info!(grace_secs = config.shutdown_grace_secs, "tessaridb-s3 stopping");
            let _receivers_gone = stop_tx.send(true);
            let grace = Duration::from_secs(config.shutdown_grace_secs);
            match tokio::time::timeout(grace, &mut server).await {
                Ok(result) => result?,
                Err(_) => tracing::warn!("grace period over; abandoning in-flight requests"),
            }
        }
        result = &mut server => result?,
    }
    tracing::info!("tessaridb-s3 stopped");
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
