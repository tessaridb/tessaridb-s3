//! The live suite leaves the test node with the namespaces it found: a test's namespace and its account go when the
//! test ends, however it ends, and a crashed run's leftovers go at the next run's start.

use tessari_s3_infrastructure::tessaridb::{Answer, MetaPool, Value};

use crate::namespaces::{name, removed_at_end, sweep};
use crate::{IGNORED, SCHEMA_TURN, live_config, prepared, scratch_dir, test_node};

/// The node's namespaces and the users' names, read as the test node's owner.
async fn catalog(owner: &MetaPool) -> (Vec<String>, Vec<String>) {
    let answers = owner
        .run_unscoped("INFO FOR STORE; INFO FOR USERS;")
        .await
        .expect("the store and its users");
    let names = |answer: Option<&Answer>, list: &str, field: Option<&str>| -> Vec<String> {
        let Some(Answer::Value {
            value: Value::Object(info),
            ..
        }) = answer
        else {
            panic!("not a value answer: {answer:?}");
        };
        let Some(Value::Array(items)) = info.get(list) else {
            panic!("no {list} list: {info:?}");
        };
        items
            .iter()
            .filter_map(|item| match (item, field) {
                (Value::String(name), None) => Some(name.clone()),
                (Value::Object(row), Some(field)) => match row.get(field) {
                    Some(Value::String(name)) => Some(name.clone()),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    };
    (
        names(answers.first(), "namespaces", None),
        names(answers.get(1), "users", Some("user")),
    )
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_test_that_panics_leaves_neither_its_namespace_nor_its_account() {
    let _why = IGNORED;
    let (sent, made) = std::sync::mpsc::channel();
    // A test of its own: its own thread and runtime, prepared, then failing.
    let failed = std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(async move {
                let config = live_config(test_node(), None, &scratch_dir());
                let _storage = prepared(&config).await;
                let namespace = config.meta.namespace.clone();
                let (namespaces, users) = catalog(&MetaPool::new(config.meta).expect("pool")).await;
                assert!(namespaces.contains(&namespace), "the namespace was made");
                assert!(
                    users.contains(&format!("svc_{namespace}")),
                    "the account was made"
                );
                sent.send(namespace).expect("sent");
                panic!("the test fails after its namespace exists");
            });
    })
    .join();
    assert!(failed.is_err(), "the inner test failed");
    let namespace = made.recv().expect("the namespace was made");
    let owner = MetaPool::new(live_config(test_node(), None, &scratch_dir()).meta).expect("pool");
    let (namespaces, users) = catalog(&owner).await;
    assert!(
        !namespaces.contains(&namespace),
        "{namespace} is still on the node"
    );
    let account = format!("svc_{namespace}");
    assert!(!users.contains(&account), "{account} is still on the node");
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn the_sweep_removes_a_gone_process_namespace_and_keeps_a_running_one() {
    let _why = IGNORED;
    let settings = live_config(test_node(), None, &scratch_dir()).meta;
    let owner = MetaPool::new(settings.clone()).expect("pool");
    // No process runs with an id above both systems' ceilings.
    let gone = format!("t_4194305_{}", uuid::Uuid::new_v4().simple());
    let running = name();
    let mut kept = settings.clone();
    kept.namespace = running.clone();
    removed_at_end(&kept);
    // Catalog definitions contend store-wide (Q-S3-2): the planting takes the turn the tests' schemas take.
    let turn = SCHEMA_TURN.lock().await;
    for namespace in [&gone, &running] {
        owner
            .run_unscoped(&format!(
                "DEFINE NAMESPACE {namespace}; USE NAMESPACE {namespace}; DEFINE DATABASE meta; \
                 USE DATABASE meta; DEFINE TABLE things (n int); DEFINE INDEX by_n ON things FIELDS n; \
                 DEFINE USER svc_{namespace} ON {namespace}.meta ROLE editor PASSWORD 'svc_{namespace}_pw';"
            ))
            .await
            .expect("planted");
    }
    drop(turn);
    let (namespaces, _) = catalog(&owner).await;
    assert!(
        namespaces.contains(&gone) && namespaces.contains(&running),
        "{namespaces:?}"
    );

    sweep(settings).await;

    let (namespaces, users) = catalog(&owner).await;
    assert!(!namespaces.contains(&gone), "{gone} is left behind");
    assert!(
        !users.contains(&format!("svc_{gone}")),
        "its account is left behind"
    );
    assert!(
        namespaces.contains(&running),
        "{running} belongs to a running process"
    );
    assert!(
        users.contains(&format!("svc_{running}")),
        "its account stays"
    );
}
