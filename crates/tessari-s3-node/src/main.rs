//! The `tessaridb-s3` process: the composition root, and nothing else.
//!
//! Order: tracing, configuration (the environment, then the settings the metadata store holds), the metadata pool
//! and schema, the daemons (the reclaimer, when a data directory is configured, the healer and the mover on a cluster
//! member, the upload reaper and the usage scanner), the API state and router, the listener — and, on a node of a
//! cluster, the internal shard listener — then serving until SIGINT or SIGTERM, after which in-flight requests and the
//! daemons' current runs get the configured grace period before the process exits.

mod console;
mod health;
mod internal;
mod keys;
mod membership;
mod metrics;
mod settings;

use std::time::Duration;

use std::sync::Arc;
use tessari_s3_api::{ApiState, serve_s3};
use tessari_s3_constants::CONSOLE_OIDC_REFRESH_MIN_SECS;
use tessari_s3_infrastructure::NotifyEndpoint;
use tessari_s3_infrastructure::oidc::OidcClient;
use tessari_s3_infrastructure::tessaridb::MetaPool;
use tessari_s3_infrastructure::tls::listener_config;

use tessari_s3_core::notification::DATABASE_TARGET;
use tessari_s3_daemons::{
    Destination, Healer, ImageWorker, LifecycleWorker, MediaPlanner, Mover, NotifyWorker,
    Reclaimer, Replicator, Rewrapper, UploadReaper, UsageScanner, VideoWorker, run,
};
use tessari_s3_storage::Storage;
use tessari_s3_storage::media::TaskQueue;
use tokio::sync::watch;
use tokio::task::JoinSet;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    // The container's health check: no shell in the image, so the binary asks itself.
    if std::env::args().nth(1).as_deref() == Some("health") {
        return health::run().await;
    }
    tessari_s3_observability::init();
    let config = settings::configure(&|key: &str| std::env::var(key).ok()).await?;
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
        Some(cluster) => {
            Storage::clustered(pool, &config.region, config.data_dir.clone(), cluster)?
        }
        None => Storage::new(pool, &config.region, config.data_dir.clone())?,
    }
    .with_iam_key(config.iam_key.clone())
    .with_reclaim_grace(config.reclaim_grace_secs)
    .with_data_floor(config.data_min_free_bytes);
    // The schema is applied before the listener opens: a node whose metadata store cannot be reached does not
    // start, rather than answering every request 503.
    storage.prepare().await?;
    tracing::info!(meta = ?config.meta.addresses, namespace = %config.meta.namespace, "metadata schema ready");
    if let Some(passphrase) = &config.keys_passphrase {
        keys::unseal_at_start(&storage, passphrase.expose()).await;
    }
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
        membership::take_place(cluster, &storage).await?;
        tracing::info!(node = cluster.node.as_str(), zone = cluster.zone.as_ref().map(|zone| zone.as_str()), code = %format_args!("{}+{}", cluster.code.data(), cluster.code.parity()), "registered with the cluster");
    }
    // Built before the daemons start, so their runs are counted in the metrics it serves.
    let state = ApiState::new(&config, ApiState::system_clock(), storage.clone());
    let counted = Arc::clone(state.metrics());
    let (stop_tx, stop_rx) = watch::channel(false);
    let mut daemons = JoinSet::new();
    let reclaimer = Reclaimer::new(
        storage.objects().clone(),
        config.reclaim_grace_secs,
        Duration::from_secs(config.reclaim_interval_secs),
    );
    daemons.spawn(run(
        Arc::new(reclaimer),
        stop_rx.clone(),
        Arc::clone(&counted),
    ));
    if let Some(cluster) = &config.cluster {
        let healer = Healer::new(
            storage.objects().clone(),
            Duration::from_secs(cluster.heal_interval_secs),
        );
        daemons.spawn(run(Arc::new(healer), stop_rx.clone(), Arc::clone(&counted)));
        let mover = Mover::new(
            storage.objects().clone(),
            Duration::from_secs(cluster.heal_interval_secs),
            config.reclaim_grace_secs,
            cluster.move_bytes_per_sec,
            stop_rx.clone(),
        );
        daemons.spawn(run(Arc::new(mover), stop_rx.clone(), Arc::clone(&counted)));
    }
    // Uploads can be opened without a data directory, so the reaper always runs.
    let reaper = UploadReaper::new(
        storage.multipart().clone(),
        config.upload_max_age_secs,
        Duration::from_secs(config.reclaim_interval_secs),
    );
    daemons.spawn(run(Arc::new(reaper), stop_rx.clone(), Arc::clone(&counted)));
    // Due buckets are looked for as often as uploads are reaped; the queue decides which are due.
    let lifecycle = LifecycleWorker::new(
        storage.lifecycle(),
        Duration::from_secs(config.reclaim_interval_secs),
    );
    daemons.spawn(run(
        Arc::new(lifecycle),
        stop_rx.clone(),
        Arc::clone(&counted),
    ));
    // Replication: this node records where its region's S3 surface is reached, then sends the region's queued
    // versions through the group, every node taking turns on it.
    let replication = storage.replication();
    if let Some(endpoint) = &config.advertise {
        replication.advertise(&config.region, endpoint).await?;
    }
    replication.define().await?;
    let replicator = Replicator::new(
        storage.clone(),
        tessari_s3_infrastructure::ReplicaClient::new()?,
    );
    daemons.spawn(run(
        Arc::new(replicator),
        stop_rx.clone(),
        Arc::clone(&counted),
    ));
    // Each declared destination reads the event topic through its own group, on every node: the group's record
    // takes the nodes in turns, so an event goes to one of them at a time.
    // The database's own topic is the default destination and always delivered to; declared ones are alternatives.
    let notifications = storage.notifications();
    notifications
        .define(
            std::iter::once(DATABASE_TARGET).chain(
                config
                    .notify_targets
                    .iter()
                    .map(|target| target.id.as_str()),
            ),
        )
        .await?;
    let default_worker = NotifyWorker::new(
        notifications.clone(),
        DATABASE_TARGET.to_owned(),
        Destination::Topic(notifications.clone()),
        config.region.clone(),
    );
    daemons.spawn(run(
        Arc::new(default_worker),
        stop_rx.clone(),
        Arc::clone(&counted),
    ));
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
            daemons.spawn(run(Arc::new(worker), stop_rx.clone(), Arc::clone(&counted)));
        }
    }
    // Every node plans the renditions of the objects the media group hands it and renders image tasks from the shared
    // queue; the consumer name says which node holds a task.
    let media = storage.media();
    media.define().await?;
    let planner = MediaPlanner::new(media.clone(), storage.objects().clone());
    daemons.spawn(run(
        Arc::new(planner),
        stop_rx.clone(),
        Arc::clone(&counted),
    ));
    let consumer = config.cluster.as_ref().map_or_else(
        || "single".to_owned(),
        |cluster| cluster.node.as_str().to_owned(),
    );
    // A node back under its own name takes back what it held when it stopped, rather than waiting the holds out.
    for queue in [TaskQueue::Images, TaskQueue::Videos] {
        media.reclaim(queue, &consumer).await?;
    }
    for _ in 0..config.media_workers {
        let worker = ImageWorker::new(media.clone(), storage.objects().clone(), consumer.clone());
        daemons.spawn(run(Arc::new(worker), stop_rx.clone(), Arc::clone(&counted)));
    }
    // Only a node with ffmpeg takes video and poster tasks; the others leave them in the queue for it.
    if let Some(ffmpeg) = &config.ffmpeg {
        let worker = VideoWorker::new(
            media.clone(),
            storage.objects().clone(),
            consumer.clone(),
            tessari_s3_infrastructure::media_video::Ffmpeg::new(ffmpeg.clone()),
            std::env::temp_dir(),
        );
        daemons.spawn(run(Arc::new(worker), stop_rx.clone(), Arc::clone(&counted)));
    }
    // After a rotation, each node re-wraps what is still under an older server key it holds.
    let rewrapper = Rewrapper::new(storage.objects().clone());
    daemons.spawn(run(
        Arc::new(rewrapper),
        stop_rx.clone(),
        Arc::clone(&counted),
    ));
    let scanner = UsageScanner::new(
        storage.usage().clone(),
        config.cluster.as_ref().map(|cluster| &cluster.node),
        Duration::from_secs(config.usage_interval_secs),
    );
    daemons.spawn(run(
        Arc::new(scanner),
        stop_rx.clone(),
        Arc::clone(&counted),
    ));
    let internal = internal::surface(&config, &storage, stop_rx.clone()).await?;
    let console = console::surface(&config, &storage, stop_rx.clone()).await?;
    // People the console's identity provider vouches for may ask the S3 surface for temporary credentials.
    if let Some(console) = &config.console
        && let Some(settings) = &console.oidc
    {
        let client = OidcClient::new(settings, CONSOLE_OIDC_REFRESH_MIN_SECS)?;
        state.accept_web_identity(client, console.sign_ins_per_minute);
    }
    let metrics = metrics::surface(&config, &state, stop_rx.clone()).await?;
    if let Some(writer) = metrics::access_log(&config, &state, stop_rx.clone()) {
        daemons.spawn(writer);
    }
    let tls = config
        .tls
        .as_ref()
        .map(|tls| listener_config(tls, "TESSARIDB_S3_TLS_CERT", "TESSARIDB_S3_TLS_KEY"))
        .transpose()?;
    let listener = tokio::net::TcpListener::bind(config.listen).await?;
    tracing::info!(listen = %config.listen, tls = tls.is_some(), "S3 API listening");
    let mut server_stop = stop_rx;
    let s3 = serve_s3(listener, state, tls, async move {
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
        let metrics = async move {
            match metrics {
                Some(metrics) => metrics.await,
                None => Ok(()),
            }
        };
        tokio::try_join!(s3, internal, console, metrics).map(|_| ())
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
