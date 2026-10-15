//! The kill-a-region drill: a writer runs in one region while its versions replicate to another; the region is killed
//! (its server, replicator and writer stop at once), an operator fails it over to the other, the writer carries on
//! there, and the killed region comes back. It measures what a failover costs — the versions acknowledged and not yet
//! replicated when the region died, and the time until the promoted region acknowledged a write — and proves the
//! returning region refuses client writes, catches up both ways, and opens again on restore.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tessari_s3_api::{ApiState, ConsoleState};
use tessari_s3_core::authz::Role;
use tessari_s3_daemons::{Daemon, Replicator};
use tessari_s3_infrastructure::{ReplicaClient, S3Config};
use tokio::task::JoinHandle;

use crate::access::user_with_key;
use crate::console::{send, sign_in};
use crate::replication_regions::{bucket, replicate};
use crate::{IGNORED, call, live_config, prepared, scratch_dir, test_node};

const EAST: &str = "us-east-1";
const WEST: &str = "eu-west-1";
/// How long the writer runs before the region is killed.
const BEFORE_THE_KILL: Duration = Duration::from_millis(1500);
/// How long catching up may take: a task that failed while the region was down is tried again after 5, 10, 20 s.
const CAUGHT_UP_WITHIN: Duration = Duration::from_secs(90);

/// A version the writer had acknowledged: its key, its id, and when the answer came.
type Acknowledged = Arc<Mutex<Vec<(String, String, Instant)>>>;

/// Serves `state`'s router on a port of its own and advertises it as its region's S3 surface.
async fn serve(state: &ApiState) -> JoinHandle<()> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a port");
    let endpoint = format!("http://{}", listener.local_addr().expect("an address"));
    let app = tessari_s3_api::router(state.clone());
    let server = tokio::spawn(async move {
        let _served = axum::serve(listener, app).await;
    });
    let replication = state.storage().replication();
    replication
        .advertise(state.region(), &endpoint)
        .await
        .expect("advertised");
    replication.define().await.expect("the group");
    server
}

/// Runs a replicator for `state`'s region every 20 ms until aborted.
fn replicating(state: &ApiState) -> JoinHandle<()> {
    let replicator = Replicator::new(
        state.storage().clone(),
        ReplicaClient::new().expect("a client"),
    );
    tokio::spawn(async move {
        loop {
            let _ran = replicator.run_once().await;
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
}

/// Writes `<prefix>NNNNN` into `/<bucket>/logs/` one after another until aborted, keeping what was acknowledged.
fn writing(
    state: &ApiState,
    bucket: &'static str,
    prefix: &'static str,
    done: Acknowledged,
) -> JoinHandle<()> {
    let state = state.clone();
    tokio::spawn(async move {
        // Far more writes than the drill gives it time for; the task is aborted long before.
        for n in 0..1_000_000_u32 {
            let key = format!("logs/{prefix}{n:05}");
            let seen = call(
                &state,
                "PUT",
                &format!("/{bucket}/{key}"),
                vec![],
                key.as_bytes(),
            )
            .await;
            if seen.status == 200 {
                let id = seen
                    .headers
                    .get("x-amz-version-id")
                    .and_then(|v| v.to_str().ok())
                    .expect("a version id");
                done.lock()
                    .expect("unpoisoned")
                    .push((key, id.to_owned(), Instant::now()));
            }
        }
    })
}

/// The acknowledged versions `state` does not hold in `bucket`.
async fn missing(
    state: &ApiState,
    bucket: &str,
    versions: &[(String, String, Instant)],
) -> Vec<(String, Instant)> {
    let mut absent = Vec::new();
    for (key, id, at) in versions {
        let seen = call(
            state,
            "HEAD",
            &format!("/{bucket}/{key}"),
            vec![("versionId", Some(id))],
            b"",
        )
        .await;
        if seen.status != 200 {
            absent.push((key.clone(), *at));
        }
    }
    absent
}

async fn node(config: &S3Config) -> ApiState {
    ApiState::new(config, ApiState::system_clock(), prepared(config).await)
}

fn snapshot(done: &Acknowledged) -> Vec<(String, String, Instant)> {
    done.lock().expect("unpoisoned").clone()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_region_killed_under_a_writer_is_failed_over_and_refuses_writes_when_it_returns() {
    let _why = IGNORED;
    let east_config = live_config(test_node(), None, &scratch_dir());
    let mut west_config = east_config.clone();
    west_config.region = WEST.to_owned();
    west_config.data_dir = scratch_dir();
    let east = node(&east_config).await;
    let west = node(&west_config).await;
    assert_eq!((east.region(), west.region()), (EAST, WEST));
    let console = ConsoleState::new(
        &west_config,
        ApiState::system_clock(),
        west.storage().clone(),
    )
    .expect("built")
    .expect("a console is configured");
    let east_server = serve(&east).await;
    let _west_server = serve(&west).await;
    let _role = user_with_key(
        east.storage(),
        "replicator",
        "default",
        Role::SpaceAdmin,
        false,
    )
    .await;
    bucket(&east, "origin").await;
    bucket(&west, "copies").await;
    replicate(&east, "origin", "copies").await;
    replicate(&west, "copies", "origin").await;
    let east_replicator = replicating(&east);
    let _west_replicator = replicating(&west);
    let from_east: Acknowledged = Arc::default();
    let east_writer = writing(&east, "origin", "e", Arc::clone(&from_east));
    tokio::time::sleep(BEFORE_THE_KILL).await;

    // The kill: the region's server, its replicator and the writer's connection to it stop at once.
    for task in [&east_writer, &east_replicator, &east_server] {
        task.abort();
    }
    let killed = Instant::now();
    drop(east);
    let acknowledged = snapshot(&from_east);
    assert!(
        !acknowledged.is_empty(),
        "the writer wrote nothing before the kill"
    );
    let unreplicated = missing(&west, "copies", &acknowledged).await;
    let oldest = unreplicated
        .iter()
        .map(|(_, at)| killed.saturating_duration_since(*at))
        .max();

    let token = sign_in(&console).await;
    let asked = Instant::now();
    let body = format!(r#"{{"region":"{EAST}","promoted":"{WEST}","reason":"drill"}}"#);
    let failed_over = send(
        &console,
        "POST",
        "/api/v1/regions/failover",
        Some(&token),
        Some(&body),
    )
    .await;
    assert_eq!(failed_over.status, 200, "{:?}", failed_over.json());
    let recorded = asked.elapsed();
    let from_west: Acknowledged = Arc::default();
    let west_writer = writing(&west, "copies", "w", Arc::clone(&from_west));
    let first_on_west = loop {
        if let Some((_, _, at)) = snapshot(&from_west).first() {
            break at.saturating_duration_since(killed);
        }
        assert!(
            killed.elapsed() < CAUGHT_UP_WITHIN,
            "the promoted region took no write"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    west_writer.abort();
    let written_west = snapshot(&from_west);

    // The former primary returns over the same data and metadata: it refuses client writes, and catches up both ways.
    let east = node(&east_config).await;
    let _east_server = serve(&east).await;
    let refused = call(&east, "PUT", "/origin/logs/late", vec![], b"late").await;
    assert_eq!(refused.status, 503, "{}", refused.body);
    assert!(refused.body.contains(WEST), "{}", refused.body);
    let _east_replicator = replicating(&east);
    let deadline = Instant::now();
    let (still_missing, held) = loop {
        let progress = east
            .storage()
            .replication()
            .progress()
            .await
            .expect("progress");
        let absent = missing(&west, "copies", &acknowledged).await;
        let caught_up = missing(&east, "origin", &written_west).await.is_empty();
        // Settled when every version still missing belongs to a task the killed replicator holds until its lease
        // lapses (the group's lag counts every position past the oldest unsettled one, so it stays up meanwhile).
        if caught_up && u64::try_from(absent.len()).is_ok_and(|absent| absent <= progress.in_flight)
        {
            break (absent, progress.in_flight);
        }
        assert!(
            deadline.elapsed() < CAUGHT_UP_WITHIN,
            "not caught up: {} missing, {progress:?}",
            absent.len()
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    assert!(
        held <= 4,
        "more held than one replicator takes at once: {held}"
    );

    let back = format!(r#"{{"region":"{EAST}","reason":"recovered"}}"#);
    let restored = send(
        &console,
        "POST",
        "/api/v1/regions/restore",
        Some(&token),
        Some(&back),
    )
    .await;
    assert_eq!(restored.status, 200, "{:?}", restored.json());
    assert_eq!(
        call(&east, "PUT", "/origin/logs/after", vec![], b"after")
            .await
            .status,
        200
    );

    println!(
        "region drill: {} versions acknowledged in {EAST} before the kill; {} not yet in {WEST} at the kill (oldest {} ms); \
         failover recorded in {} ms; first write acknowledged in {WEST} {} ms after the kill; on return {} delivered at \
         once and {} held by the killed replicator's leases until their deadline; {} written in {WEST} meanwhile, all \
         replicated back",
        acknowledged.len(),
        unreplicated.len(),
        oldest.map_or(0, |age| age.as_millis()),
        recorded.as_millis(),
        first_on_west.as_millis(),
        unreplicated.len().saturating_sub(still_missing.len()),
        still_missing.len(),
        written_west.len(),
    );
}
