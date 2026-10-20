//! What a node served is in its metrics: a write, a read and a read of a missing key, each counted by operation,
//! status and code, and timed in the size class of the bytes it moved.

use axum::body::{Body, to_bytes};
use axum::http::Request;
use tessari_s3_api::{ApiState, metrics_router};
use tower::ServiceExt;

use tessari_s3_core::authz::Role;

use crate::access::user_with_key;
use crate::{IGNORED, call, call_as, fresh};

async fn scrape(state: ApiState) -> String {
    let response = metrics_router(state)
        .oneshot(
            Request::get("/metrics")
                .body(Body::empty())
                .expect("a request"),
        )
        .await
        .expect("the router is infallible");
    String::from_utf8(
        to_bytes(response.into_body(), 1 << 20)
            .await
            .expect("body")
            .to_vec(),
    )
    .expect("utf-8")
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn served_requests_are_counted_by_operation_status_code_and_size() {
    let _why = IGNORED;
    let (state, _) = fresh().await;
    assert_eq!(
        call(&state, "PUT", "/counted", vec![], b"").await.status,
        200
    );
    let body = vec![7_u8; 100 << 10];
    let put = call(&state, "PUT", "/counted/one", vec![], &body).await;
    assert_eq!(put.status, 200, "{}", put.body);
    assert_eq!(
        call(&state, "GET", "/counted/one", vec![], b"")
            .await
            .status,
        200
    );
    let missing = call(&state, "GET", "/counted/none", vec![], b"").await;
    assert_eq!(missing.status, 404);
    let text = scrape(state).await;
    for line in [
        "s3_requests_total{operation=\"CreateBucket\",status=\"200\",code=\"None\"} 1\n",
        "s3_requests_total{operation=\"PutObject\",status=\"200\",code=\"None\"} 1\n",
        "s3_requests_total{operation=\"GetObject\",status=\"200\",code=\"None\"} 1\n",
        "s3_requests_total{operation=\"GetObject\",status=\"404\",code=\"NoSuchKey\"} 1\n",
        // The write's body and the read's answer are 100 KiB; the miss moved only its error document.
        "s3_request_duration_seconds_count{operation=\"PutObject\",size=\"lt_1MiB\"} 1\n",
        "s3_request_duration_seconds_count{operation=\"GetObject\",size=\"lt_1MiB\"} 1\n",
        "s3_request_duration_seconds_count{operation=\"GetObject\",size=\"lt_64KiB\"} 1\n",
    ] {
        assert!(text.contains(line), "missing {line}in\n{text}");
    }
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_policy_denial_is_counted_and_the_replication_backlog_is_read() {
    let _why = IGNORED;
    let (state, _) = fresh().await;
    assert_eq!(
        call(&state, "PUT", "/guarded", vec![], b"").await.status,
        200
    );
    let bob = user_with_key(state.storage(), "bob", "alpha", Role::Member, false).await;
    let denied = call_as(
        &state,
        bob.pair(),
        "GET",
        "/guarded/one",
        vec![],
        vec![],
        b"",
    )
    .await;
    assert_eq!(
        denied.code.as_deref(),
        Some("AccessDenied"),
        "{}",
        denied.body
    );
    let text = scrape(state).await;
    for line in [
        "s3_policy_denials_total{operation=\"GetObject\"} 1\n",
        "s3_collect_up{source=\"replication\"} 1\n",
        "s3_replication_backlog{} 0\n",
        "s3_replication_failed{} 0\n",
    ] {
        assert!(text.contains(line), "missing {line}in\n{text}");
    }
    // A node outside an erasure-coded cluster heals nothing and has no members, so it reports neither.
    assert!(!text.contains("s3_heal_backlog"), "{text}");
    assert!(!text.contains("s3_cluster_members{"), "{text}");
    assert!(
        text.contains("s3_collect_up{source=\"cluster\"} 1\n"),
        "{text}"
    );
    assert!(!text.contains("s3_auth_refusals_total{"), "{text}");
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn the_cluster_members_and_those_answering_are_counted() {
    let _why = IGNORED;
    let (state, _) = fresh().await;
    // Registered at addresses nothing listens on: members that do not answer.
    for name in ["n1", "n2"] {
        let node = tessari_s3_types::NodeId::new(name).expect("a node id");
        crate::joined(state.storage(), &node, "127.0.0.1:1", None).await;
    }
    let text = scrape(state).await;
    for line in [
        "s3_cluster_members{} 2\n",
        "s3_cluster_members_answering{} 0\n",
        "s3_collect_up{source=\"cluster\"} 1\n",
    ] {
        assert!(text.contains(line), "missing {line}in\n{text}");
    }
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn the_nodes_state_and_health_are_read_at_the_scrape() {
    let _why = IGNORED;
    let (state, _) = fresh().await;
    assert_eq!(call(&state, "PUT", "/held", vec![], b"").await.status, 200);
    for key in ["/held/a", "/held/b"] {
        assert_eq!(call(&state, "PUT", key, vec![], b"12345").await.status, 200);
    }
    let created = call(&state, "POST", "/held/big", vec![("uploads", None)], b"").await;
    assert_eq!(created.status, 200, "{}", created.body);
    state
        .storage()
        .usage()
        .refresh("standalone")
        .await
        .expect("measured");
    let text = scrape(state).await;
    let value = |name: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(name))
            .and_then(|rest| rest.trim().parse::<f64>().ok())
            .unwrap_or_else(|| panic!("{name} missing in\n{text}"))
    };
    for line in [
        format!(
            "s3_build_info{{version=\"{}\"}} 1\n",
            env!("CARGO_PKG_VERSION")
        ),
        "s3_collect_up{source=\"meta\"} 1\n".to_owned(),
        "s3_collect_up{source=\"keys\"} 1\n".to_owned(),
        "s3_collect_up{source=\"drive\"} 1\n".to_owned(),
        "s3_collect_up{source=\"usage\"} 1\n".to_owned(),
        "s3_collect_up{source=\"uploads\"} 1\n".to_owned(),
        // The suite's stores never create a keys vault.
        "s3_keys_vault{state=\"uninitialised\"} 1\n".to_owned(),
        "s3_keys_vault{state=\"sealed\"} 0\n".to_owned(),
        "s3_buckets{} 1\n".to_owned(),
        "s3_objects{} 2\n".to_owned(),
        "s3_stored_bytes{} 10\n".to_owned(),
        "s3_uploads_open{} 1\n".to_owned(),
    ] {
        assert!(text.contains(&line), "missing {line}in\n{text}");
    }
    let probe = value("s3_meta_probe_seconds{}");
    assert!(probe > 0.0 && probe < 5.0, "{probe}");
    let capacity = value("s3_data_capacity_bytes{}");
    let free = value("s3_data_free_bytes{}");
    assert!(capacity > 0.0 && free <= capacity, "{free} of {capacity}");
    assert!(value("s3_start_time_seconds") > 1.7e9);
    assert!(value("s3_usage_taken_seconds{}") >= value("s3_start_time_seconds") - 60.0);
}
