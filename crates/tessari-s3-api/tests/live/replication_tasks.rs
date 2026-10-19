//! The replicators' side of the outbox against a real node: the group hands each queued version out once, a sent one
//! is stamped `COMPLETED` and settled, a failed one waits before it is handed out again and on its last hand-out is
//! stamped `FAILED`; each region's S3 endpoint is kept in the global registry.

use tessari_s3_constants::{REPLICATION_DELIVERIES, REPLICATION_TAKE as REPLICATOR_TAKE};

use crate::replication::{ENABLED, configuration};
use tessari_s3_api::ApiState;

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
    replicated_origin(&state).await;
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

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_replicator_that_dies_holding_its_tasks_leaves_the_rest_to_another() {
    let _why = IGNORED;
    let (state, _planter) = fresh().await;
    replicated_origin(&state).await;
    let replication = state.storage().replication();
    replication.define().await.expect("the group");
    for n in 0..3 * REPLICATOR_TAKE {
        let path = format!("/origin/logs/{n}");
        assert_eq!(call(&state, "PUT", &path, vec![], b"x").await.status, 200);
    }
    // The first replicator takes what one run takes and never answers, as one killed mid-run.
    let held = replication.next(REPLICATOR_TAKE).await.expect("tasks");
    assert_eq!(held.len(), REPLICATOR_TAKE, "{held:?}");
    let other = replication.next(REPLICATOR_TAKE).await.expect("tasks");
    assert_eq!(
        other.len(),
        REPLICATOR_TAKE,
        "the rest is handed out while the dead one holds its tasks"
    );
    assert!(
        other
            .iter()
            .all(|task| held.iter().all(|kept| kept.key != task.key)),
        "{other:?}"
    );
}

/// The versioned buckets `origin` and `copies`, `origin` replicating everything to `copies`.
async fn replicated_origin(state: &ApiState) {
    for bucket in ["origin", "copies"] {
        assert_eq!(
            call(state, "PUT", &format!("/{bucket}"), vec![], b"")
                .await
                .status,
            200
        );
        assert_eq!(
            call(
                state,
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
        state,
        "PUT",
        "/origin",
        vec![("replication", None)],
        configuration("copies", "").as_bytes(),
    )
    .await;
    assert_eq!(configured.status, 200, "{}", configured.body);
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_group_an_earlier_release_declared_narrower_is_kept_and_named() {
    let _why = IGNORED;
    let (state, planter) = fresh().await;
    planter
        .run(
            "DEFINE GROUP 'replicators' ON TOPIC replication_tasks ACK DEADLINE 300s DELIVERIES 10 IN FLIGHT 4 \
             DEAD LETTER TO replication_failed;",
            vec![],
        )
        .await
        .expect("declared as 0.3.0 declared it");
    // The declaration is read back to compare widths; the group itself cannot be altered.
    state
        .storage()
        .replication()
        .define()
        .await
        .expect("defined over it");
    let info = planter
        .run("INFO FOR TOPIC replication_tasks;", vec![])
        .await
        .expect("info");
    let text = format!("{info:?}");
    assert!(text.contains("width"), "{text}");
    assert!(
        text.contains("Integer(4)") && !text.contains("Integer(64)"),
        "the earlier width is kept: {text}"
    );
}
