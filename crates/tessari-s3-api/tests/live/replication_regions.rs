//! Replication between two regions over one namespace, both ways: each region's version reaches the other through
//! that region's advertised S3 surface, and a replica is never sent back — each version is sent once, and the loop a
//! two-way configuration invites does not start.

use tessari_s3_api::ApiState;
use tessari_s3_core::authz::Role;
use tessari_s3_daemons::{Daemon, Replicator};
use tessari_s3_infrastructure::ReplicaClient;

use crate::access::user_with_key;
use crate::replication::{ENABLED, configuration};
use crate::{IGNORED, Seen, call, live_config, prepared, scratch_dir, test_node};

const EAST: &str = "us-east-1";
const WEST: &str = "eu-west-1";

fn header<'a>(seen: &'a Seen, name: &str) -> Option<&'a str> {
    seen.headers.get(name).and_then(|value| value.to_str().ok())
}

/// A node of `region`, serving on a port of its own that it advertises for its region.
async fn serving(state: &ApiState) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a port");
    let endpoint = format!("http://{}", listener.local_addr().expect("an address"));
    let app = tessari_s3_api::router(state.clone());
    tokio::spawn(async move { axum::serve(listener, app).await });
    let replication = state.storage().replication();
    replication
        .advertise(state.region(), &endpoint)
        .await
        .expect("advertised");
    replication.define().await.expect("the group");
}

/// A versioned bucket made on `state`'s region, replicating every `logs/` key to `destination`.
async fn bucket(state: &ApiState, name: &str) {
    assert_eq!(
        call(state, "PUT", &format!("/{name}"), vec![], b"")
            .await
            .status,
        200
    );
    assert_eq!(
        call(
            state,
            "PUT",
            &format!("/{name}"),
            vec![("versioning", None)],
            ENABLED
        )
        .await
        .status,
        200
    );
}

async fn replicate(state: &ApiState, name: &str, destination: &str) {
    let rule = configuration(destination, "").replace(
        "arn:aws:iam::000000000000:role/replication",
        "arn:aws:iam::000000000000:role/replicator",
    );
    let configured = call(
        state,
        "PUT",
        &format!("/{name}"),
        vec![("replication", None)],
        rule.as_bytes(),
    )
    .await;
    assert_eq!(configured.status, 200, "{}", configured.body);
}

async fn run(replicator: &Replicator) -> String {
    replicator.run_once().await.expect("a run").message
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn two_regions_replicate_both_ways_and_never_send_a_replica_back() {
    let _why = IGNORED;
    let east_config = live_config(test_node(), None, &scratch_dir());
    let mut west_config = east_config.clone();
    west_config.region = WEST.to_owned();
    west_config.data_dir = scratch_dir();
    let east = ApiState::new(
        &east_config,
        ApiState::system_clock(),
        prepared(&east_config).await,
    );
    let west = ApiState::new(
        &west_config,
        ApiState::system_clock(),
        prepared(&west_config).await,
    );
    assert_eq!((east.region(), west.region()), (EAST, WEST));
    serving(&east).await;
    serving(&west).await;
    let _role = user_with_key(
        east.storage(),
        "replicator",
        "default",
        Role::SpaceAdmin,
        false,
    )
    .await;
    bucket(&east, "east").await;
    bucket(&west, "west").await;
    replicate(&east, "east", "west").await;
    replicate(&west, "west", "east").await;
    let client = || ReplicaClient::new().expect("a client");
    let east_replicator = Replicator::new(east.storage().clone(), client());
    let west_replicator = Replicator::new(west.storage().clone(), client());

    let written = call(&east, "PUT", "/east/logs/a", vec![], b"from the east").await;
    assert_eq!(written.status, 200, "{}", written.body);
    assert_eq!(run(&east_replicator).await, "replicated 1, to retry 0");
    let arrived = call(&west, "GET", "/west/logs/a", vec![], b"").await;
    assert_eq!(
        (
            arrived.body.as_str(),
            header(&arrived, "x-amz-replication-status"),
            header(&arrived, "x-amz-version-id")
        ),
        (
            "from the east",
            Some("REPLICA"),
            header(&written, "x-amz-version-id")
        )
    );
    assert_eq!(
        run(&west_replicator).await,
        "replicated 0, to retry 0",
        "the replica is not sent back"
    );

    let written = call(&west, "PUT", "/west/logs/b", vec![], b"from the west").await;
    assert_eq!(written.status, 200, "{}", written.body);
    assert_eq!(run(&west_replicator).await, "replicated 1, to retry 0");
    let arrived = call(&east, "GET", "/east/logs/b", vec![], b"").await;
    assert_eq!(
        (
            arrived.body.as_str(),
            header(&arrived, "x-amz-replication-status")
        ),
        ("from the west", Some("REPLICA"))
    );
    assert_eq!(run(&east_replicator).await, "replicated 0, to retry 0");
    let source = call(&east, "HEAD", "/east/logs/a", vec![], b"").await;
    assert_eq!(
        header(&source, "x-amz-replication-status"),
        Some("COMPLETED")
    );
}
