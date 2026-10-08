//! The `tessaridb-s3` process: the composition root, and nothing else.
//!
//! Order: tracing, configuration, the metadata pool and schema, the daemons (the reclaimer, when a data directory is
//! configured, and the upload reaper), the API state and router, the listener — and, on a node of a cluster, the
//! internal shard listener — then serving until SIGINT or SIGTERM, after which in-flight
//! requests and the daemons' current runs get the configured grace period before the process exits.

use std::time::Duration;

use std::sync::Arc;
use tessari_s3_api::{ApiState, InternalState, internal_router, router};
use tessari_s3_infrastructure::S3Config;
use tessari_s3_infrastructure::tessaridb::MetaPool;

use tessari_s3_daemons::{Reclaimer, UploadReaper, run};
use tessari_s3_storage::Storage;
use tokio::sync::watch;
use tokio::task::JoinSet;

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
    tracing::info!(meta = ?config.meta.addresses, namespace = %config.meta.namespace, "metadata schema ready");
    if let Some(cluster) = &config.cluster {
        // Registered before serving, so a node is never answering requests the cluster does not know it exists for.
        storage
            .cluster()
            .register(&cluster.node, &cluster.internal_advertise)
            .await?;
        tracing::info!(node = cluster.node.as_str(), code = %format_args!("{}+{}", cluster.code.data(), cluster.code.parity()), "registered with the cluster");
    }
    let (stop_tx, stop_rx) = watch::channel(false);
    let mut daemons = JoinSet::new();
    if storage.objects().stores_data() {
        let reclaimer = Reclaimer::new(
            storage.objects().clone(),
            config.reclaim_grace_secs,
            Duration::from_secs(config.reclaim_interval_secs),
        );
        daemons.spawn(run(Arc::new(reclaimer), stop_rx.clone()));
    }
    // Uploads can be opened without a data directory, so the reaper always runs.
    let reaper = UploadReaper::new(
        storage.multipart().clone(),
        config.upload_max_age_secs,
        Duration::from_secs(config.reclaim_interval_secs),
    );
    daemons.spawn(run(Arc::new(reaper), stop_rx.clone()));
    let internal = internal_surface(&config, &storage, stop_rx.clone()).await?;
    let app = router(ApiState::new(&config, ApiState::system_clock(), storage));
    let listener = tokio::net::TcpListener::bind(config.listen).await?;
    let mut server_stop = stop_rx;
    let s3 = axum::serve(listener, app).with_graceful_shutdown(async move {
        // A closed sender also means stop: the select below has returned.
        let _closed = server_stop.changed().await;
    });
    // Both surfaces stop on the same signal; either one failing ends the process.
    let server = async move {
        let internal = async move {
            match internal {
                Some(internal) => internal.await,
                None => Ok(()),
            }
        };
        tokio::try_join!(s3.into_future(), internal).map(|_| ())
    };
    let mut server = std::pin::pin!(server);
    tokio::select! {
        biased;
        signal = shutdown_signal() => {
            signal?;
            tracing::info!(grace_secs = config.shutdown_grace_secs, "tessaridb-s3 stopping");
            let _receivers_gone = stop_tx.send(true);
            let grace = Duration::from_secs(config.shutdown_grace_secs);
            // The listener drains and the daemons finish their current run, all within the one grace period.
            let drained = tokio::time::timeout(grace, async {
                let served = (&mut server).await;
                while daemons.join_next().await.is_some() {}
                served
            })
            .await;
            match drained {
                Ok(result) => result?,
                Err(_) => tracing::warn!("grace period over; abandoning in-flight work"),
            }
        }
        result = &mut server => result?,
    }
    tracing::info!("tessaridb-s3 stopped");
    Ok(())
}

/// On a node of a cluster, the internal shard surface: bound before anything is served, stopped with the S3
/// surface. `None` on a node of its own.
async fn internal_surface(
    config: &S3Config,
    storage: &Storage,
    mut stop: watch::Receiver<bool>,
) -> anyhow::Result<Option<impl Future<Output = std::io::Result<()>> + use<>>> {
    let Some(cluster) = &config.cluster else {
        return Ok(None);
    };
    let state = InternalState::new(
        cluster.secret.clone(),
        ApiState::system_clock(),
        storage.clone(),
        config.max_inflight,
    );
    let listener = tokio::net::TcpListener::bind(cluster.internal_listen).await?;
    tracing::info!(internal_listen = %cluster.internal_listen, "internal shard surface listening");
    let serving =
        axum::serve(listener, internal_router(state)).with_graceful_shutdown(async move {
            let _closed = stop.changed().await;
        });
    Ok(Some(serving.into_future()))
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
