//! The replicators' side of the outbox against a real node: the group hands each queued version out once, a sent one
//! is stamped `COMPLETED` and settled, a failed one waits before it is handed out again and on its last hand-out is
//! stamped `FAILED`; each region's S3 endpoint is kept in the global registry.

use tessari_s3_constants::REPLICATION_DELIVERIES;

use crate::replication::{ENABLED, configuration};
use crate::{IGNORED, Seen, call, fresh};

fn status(seen: &Seen) -> Option<&str> {
    seen.headers
        .get("x-amz-replication-status")
        .and_then(|value| value.to_str().ok())
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_task_is_handed_out_once_and_its_version_stamped_completed_or_failed() {
    let _why = IGNORED;
    let (state, _planter) = fresh().await;
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
    let configured = call(
        &state,
        "PUT",
        "/origin",
        vec![("replication", None)],
        configuration("copies", "").as_bytes(),
    )
    .await;
    assert_eq!(configured.status, 200, "{}", configured.body);
    let replication = state.storage().replication();
    replication.define().await.expect("the group");

    assert_eq!(
        call(&state, "PUT", "/origin/logs/a", vec![], b"a")
            .await
            .status,
        200
    );
    let queued = replication.progress().await.expect("progress");
    assert_eq!((queued.backlog, queued.in_flight), (1, 0), "{queued:?}");
    let handed = replication.next(10).await.expect("tasks");
    assert_eq!(handed.len(), 1, "{handed:?}");
    let held = replication.progress().await.expect("progress");
    assert_eq!(
        (held.backlog, held.in_flight, held.failed),
        (1, 1, 0),
        "{held:?}"
    );
    let first = &handed[0];
    assert_eq!(
        (
            first.key.as_str(),
            first.destination.as_str(),
            first.marker,
            first.deliveries
        ),
        ("logs/a", "copies", false, 1)
    );
    assert!(
        replication.next(10).await.expect("tasks").is_empty(),
        "held, not handed out twice"
    );
    replication.completed(first).await.expect("settled");
    let head = call(&state, "HEAD", "/origin/logs/a", vec![], b"").await;
    assert_eq!(status(&head), Some("COMPLETED"));

    assert_eq!(
        call(&state, "PUT", "/origin/logs/b", vec![], b"b")
            .await
            .status,
        200
    );
    let handed = replication.next(10).await.expect("tasks");
    assert_eq!(
        handed.len(),
        1,
        "the settled task is not handed out again: {handed:?}"
    );
    let failing = &handed[0];
    replication.failed(failing).await.expect("handed back");
    assert!(
        replication.next(10).await.expect("tasks").is_empty(),
        "a failed task waits before it is tried again"
    );
    let head = call(&state, "HEAD", "/origin/logs/b", vec![], b"").await;
    assert_eq!(
        status(&head),
        Some("PENDING"),
        "not given up on its first try"
    );
    let last = tessari_s3_storage::replication::Task {
        deliveries: u64::from(REPLICATION_DELIVERIES),
        ..failing.clone()
    };
    replication.failed(&last).await.expect("given up");
    let head = call(&state, "HEAD", "/origin/logs/b", vec![], b"").await;
    assert_eq!(status(&head), Some("FAILED"));

    assert_eq!(replication.endpoint("nowhere").await.expect("read"), None);
    replication
        .advertise("eu-test", "https://s3.example.test:9000")
        .await
        .expect("advertised");
    assert_eq!(
        replication
            .endpoint("eu-test")
            .await
            .expect("read")
            .as_deref(),
        Some("https://s3.example.test:9000")
    );
}
