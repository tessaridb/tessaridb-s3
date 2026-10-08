//! The metadata store as a cluster sees it: the namespace states its replication (a cluster refuses one that does
//! not), and a listed node that does not answer is passed over for the next.

use tessari_s3_infrastructure::tessaridb::{Answer, MetaPool, Value};

use crate::{IGNORED, call, fresh, fresh_at, test_node};

/// The namespace's own description, read through `USE` because the pool scopes every unit of work to it.
async fn namespace_info(planter: &MetaPool) -> Value {
    let (namespace, _) = planter.scope();
    let answers = planter
        .run_unscoped(&format!("USE NAMESPACE {namespace}; INFO FOR NAMESPACE;"))
        .await
        .expect("INFO FOR NAMESPACE answers");
    match answers.into_iter().nth(1) {
        Some(Answer::Value { value, .. }) => value,
        other => panic!("not a value: {other:?}"),
    }
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn the_schema_declares_the_namespaces_replication() {
    let _why = IGNORED;
    let (_, planter) = fresh().await;
    match namespace_info(&planter).await {
        Value::Object(fields) => assert_eq!(
            fields.get("replication"),
            Some(&Value::String("none".to_owned())),
            "declined out loud, not left unsaid: {fields:?}"
        ),
        other => panic!("not an object: {other:?}"),
    }
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_listed_metadata_node_that_does_not_answer_is_passed_over() {
    let _why = IGNORED;
    // Nothing listens on this port during the suite: hand-run nodes use 47961/47962.
    let addresses = format!("127.0.0.1:47909,{}", test_node());
    let (state, _, _) = fresh_at(addresses, Some("none"), None).await;
    assert_eq!(
        call(&state, "PUT", "/failover", vec![], b"").await.status,
        200
    );
    let listed = call(&state, "GET", "/", vec![], b"").await;
    assert!(
        listed.body.contains("<Name>failover</Name>"),
        "{}",
        listed.body
    );
}
