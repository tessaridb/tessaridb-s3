//! The `tessaridb-s3` process: the composition root, and nothing else.
//!
//! Order: tracing, configuration, the metadata pool and schema, the daemons (the reclaimer, when a data directory is
//! configured, the healer on a cluster member, the upload reaper and the usage scanner), the API state and router,
//! the listener — and, on a node of a cluster, the internal shard listener — then serving until SIGINT or SIGTERM,
//! after which in-flight requests and the daemons' current runs get the configured grace period before the process
//! exits.

mod console;
mod internal;

use std::time::Duration;

use std::sync::Arc;
use tessari_s3_api::{ApiState, router};
use tessari_s3_infrastructure::tessaridb::MetaPool;
use tessari_s3_infrastructure::{NotifyEndpoint, S3Config};

use tessari_s3_daemons::{
    Destination, Healer, LifecycleWorker, NotifyWorker, Reclaimer, UploadReaper, UsageScanner, run,
};
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
    let pool = MetaPool::new(config.meta.clone())?;
    let storage = match &config.cluster {
        Some(cluster) => Storage::clustered(pool, config.data_dir.clone(), cluster)?,
        None => Storage::new(pool, config.data_dir.clone()),
    }
    .with_iam_key(config.iam_key.clone())
    .with_reclaim_grace(config.reclaim_grace_secs);
    // The schema is applied before the listener opens: a node whose metadata store cannot be reached does not
    // start, rather than answering every request 503.
    storage.prepare().await?;
    tracing::info!(meta = ?config.meta.addresses, namespace = %config.meta.namespace, "metadata schema ready");
    // A store from before ADR-0008 holds small objects' bytes in their records; they move to data files before the
    // node serves, and the column goes.
    let converted = storage.objects().convert_inline().await?;
    if converted > 0 {
        tracing::info!(
            converted,
            "object bytes moved from the metadata to data files"
        );
    }
    // A store from before ADR-0010 has objects and no versions; each becomes its key's null version — after the
    // bytes moved out above, because a version copies the object as it is then.
    let versioned = storage.objects().convert_versions().await?;
    if versioned > 0 {
        tracing::info!(versioned, "objects given their null version");
    }
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
    let reclaimer = Reclaimer::new(
        storage.objects().clone(),
        config.reclaim_grace_secs,
        Duration::from_secs(config.reclaim_interval_secs),
    );
    daemons.spawn(run(Arc::new(reclaimer), stop_rx.clone()));
    if let Some(cluster) = &config.cluster {
        let healer = Healer::new(
            storage.objects().clone(),
            Duration::from_secs(cluster.heal_interval_secs),
        );
        daemons.spawn(run(Arc::new(healer), stop_rx.clone()));
    }
    // Uploads can be opened without a data directory, so the reaper always runs.
    let reaper = UploadReaper::new(
        storage.multipart().clone(),
        config.upload_max_age_secs,
        Duration::from_secs(config.reclaim_interval_secs),
    );
    daemons.spawn(run(Arc::new(reaper), stop_rx.clone()));
    // Due buckets are looked for as often as uploads are reaped; the queue decides which are due.
    let lifecycle = LifecycleWorker::new(
        storage.lifecycle(),
        Duration::from_secs(config.reclaim_interval_secs),
    );
    daemons.spawn(run(Arc::new(lifecycle), stop_rx.clone()));
    // Each declared destination reads the event topic through its own group, on every node: the group's record
    // takes the nodes in turns, so an event goes to one of them at a time.
    let notifications = storage.notifications();
    notifications
        .define(
            config
                .notify_targets
                .iter()
                .map(|target| target.id.as_str()),
        )
        .await?;
    if !config.notify_targets.is_empty() {
        let client = tessari_s3_infrastructure::WebhookClient::new()?;
        for target in &config.notify_targets {
            let destination = match &target.endpoint {
                NotifyEndpoint::Webhook(url) => Destination::Webhook {
                    url: url.clone(),
                    client: client.clone(),
                },
                NotifyEndpoint::Kafka { brokers, topic } => Destination::Kafka(
                    tessari_s3_infrastructure::KafkaProducer::new(brokers.clone(), topic.clone()),
                ),
            };
            let worker = NotifyWorker::new(
                notifications.clone(),
                target.id.clone(),
                destination,
                config.region.clone(),
            );
            daemons.spawn(run(Arc::new(worker), stop_rx.clone()));
        }
    }
    let scanner = UsageScanner::new(
        storage.usage().clone(),
        config.cluster.as_ref().map(|cluster| &cluster.node),
        Duration::from_secs(config.usage_interval_secs),
    );
    daemons.spawn(run(Arc::new(scanner), stop_rx.clone()));
    let internal = internal::surface(&config, &storage, stop_rx.clone()).await?;
    let console = console::surface(&config, &storage, stop_rx.clone()).await?;
    let app = router(ApiState::new(&config, ApiState::system_clock(), storage));
    let listener = tokio::net::TcpListener::bind(config.listen).await?;
    let mut server_stop = stop_rx;
    let s3 = axum::serve(listener, app).with_graceful_shutdown(async move {
        // A closed sender also means stop: the select below has returned.
        let _closed = server_stop.changed().await;
    });
    // Every surface stops on the same signal; any one failing ends the process.
    let server = async move {
        let internal = async move {
            match internal {
                Some(internal) => internal.await,
                None => Ok(()),
            }
        };
        let console = async move {
            match console {
                Some(console) => console.await,
                None => Ok(()),
            }
        };
        tokio::try_join!(s3.into_future(), internal, console).map(|_| ())
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
