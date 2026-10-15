//! A replicator sending versions end to end against a real node: the source bucket's version reaches the destination
//! through the destination region's advertised S3 surface, signed as the configuration's Role user, keeping its id,
//! metadata and checksum; the source is stamped `COMPLETED`; a delete marker follows when the rule says so.

use tessari_s3_core::authz::Role;
use tessari_s3_daemons::{Daemon, Replicator};
use tessari_s3_infrastructure::ReplicaClient;

use crate::access::user_with_key;
use crate::replication::{ENABLED, configuration};
use crate::{IGNORED, Seen, call, call_with, fresh_with, scratch_dir};

fn header<'a>(seen: &'a Seen, name: &str) -> Option<&'a str> {
    seen.headers.get(name).and_then(|value| value.to_str().ok())
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_version_and_its_delete_marker_reach_the_destination_as_replicas() {
    let _why = IGNORED;
    let (state, _planter, _) = fresh_with(scratch_dir()).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a port");
    let endpoint = format!("http://{}", listener.local_addr().expect("an address"));
    let app = tessari_s3_api::router(state.clone());
    tokio::spawn(async move { axum::serve(listener, app).await });
    let storage = state.storage();
    storage
        .replication()
        .advertise(state.region(), &endpoint)
        .await
        .expect("advertised");
    storage.replication().define().await.expect("the group");
    let _replicator_user =
        user_with_key(storage, "replicator", "default", Role::SpaceAdmin, false).await;

    for bucket in ["origin", "copies"] {
        assert_eq!(
            call(&state, "PUT", &format!("/{bucket}"), vec![], b"")
                .await
                .status,
            200
        );
        assert_eq!(
            call(
                &state,
                "PUT",
                &format!("/{bucket}"),
                vec![("versioning", None)],
                ENABLED
            )
            .await
            .status,
            200
        );
    }
    let rule = configuration("copies", "")
        .replace(
            "arn:aws:iam::000000000000:role/replication",
            "arn:aws:iam::000000000000:role/replicator",
        )
        .replace(
            "<DeleteMarkerReplication><Status>Disabled</Status>",
            "<DeleteMarkerReplication><Status>Enabled</Status>",
        );
    let configured = call(
        &state,
        "PUT",
        "/origin",
        vec![("replication", None)],
        rule.as_bytes(),
    )
    .await;
    assert_eq!(configured.status, 200, "{}", configured.body);

    let written = call_with(
        &state,
        "PUT",
        "/origin/logs/a",
        vec![],
        vec![("content-type", "text/plain"), ("x-amz-meta-color", "blue")],
        b"payload",
    )
    .await;
    assert_eq!(written.status, 200, "{}", written.body);
    let version = header(&written, "x-amz-version-id")
        .expect("a version id")
        .to_owned();
    let replicator = Replicator::new(storage.clone(), ReplicaClient::new().expect("a client"));
    let report = replicator.run_once().await.expect("a run");
    assert_eq!(report.message, "replicated 1, to retry 0");

    let replica = call(&state, "GET", "/copies/logs/a", vec![], b"").await;
    assert_eq!(replica.status, 200, "{}", replica.body);
    assert_eq!(replica.body, "payload");
    assert_eq!(
        (
            header(&replica, "x-amz-version-id"),
            header(&replica, "x-amz-replication-status"),
            header(&replica, "x-amz-meta-color"),
            header(&replica, "content-type"),
            header(&replica, "etag"),
        ),
        (
            Some(version.as_str()),
            Some("REPLICA"),
            Some("blue"),
            Some("text/plain"),
            header(&written, "etag"),
        )
    );
    let source = call(&state, "HEAD", "/origin/logs/a", vec![], b"").await;
    assert_eq!(
        header(&source, "x-amz-replication-status"),
        Some("COMPLETED")
    );

    let marked = call(&state, "DELETE", "/origin/logs/a", vec![], b"").await;
    assert_eq!(marked.status, 204);
    let report = replicator.run_once().await.expect("a run");
    assert_eq!(report.message, "replicated 1, to retry 0");
    assert_eq!(
        call(&state, "HEAD", "/copies/logs/a", vec![], b"")
            .await
            .status,
        404,
        "the marker hides the replica too"
    );
    let previous = call(
        &state,
        "GET",
        "/copies/logs/a",
        vec![("versionId", Some(version.as_str()))],
        b"",
    )
    .await;
    assert_eq!(
        previous.body, "payload",
        "the replica version stays behind the marker"
    );
}
