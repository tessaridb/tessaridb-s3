//! The server signs in to TessariDB as its own account, against a real node: an editor of its one database and
//! nothing else — not the owner — so its schema still applies, while a statement about another namespace or the whole
//! store is refused by the node itself.

use tessari_s3_infrastructure::tessaridb::{Answer, MetaPool};

use crate::{IGNORED, confined, live_config, prepared, scratch_dir, test_node};

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn the_server_runs_as_an_editor_of_its_one_database_and_is_refused_outside_it() {
    let _why = IGNORED;
    let config = live_config(test_node(), None, &scratch_dir());
    // The schema applies under the confined account (the harness signs every live test in this way).
    let _storage = prepared(&config).await;
    let account = confined(&config).await;
    let owner = MetaPool::new(config.meta.clone()).expect("pool");
    let (namespace, database) = owner.scope();
    let reach = format!("{namespace}.{database}");

    let answers = owner
        .run(&format!("INFO FOR USER {};", account.user), Vec::new())
        .await
        .expect("the owner reads the account");
    let Some(Answer::Value { value, .. }) = answers.into_iter().next() else {
        panic!("INFO FOR USER answered no value");
    };
    let info = format!("{value:?}");
    assert!(info.contains("\"editor\""), "{info}");
    for kind in ["read", "write", "manage"] {
        assert!(
            info.contains(&format!("\"{kind}\"")),
            "{kind} missing: {info}"
        );
    }
    for kind in ["govern", "operate", "replicate"] {
        assert!(
            !info.contains(&format!("\"{kind}\"")),
            "{kind} held: {info}"
        );
    }
    assert!(info.contains(&reach), "the reach is its database: {info}");

    let server = MetaPool::new(account).expect("pool");
    let other = server
        .run_unscoped("USE NAMESPACE some_other_namespace;")
        .await
        .expect_err("another namespace is outside its reach");
    assert!(
        other.to_string().contains("outside this user's namespace"),
        "{other}"
    );
    let store = server
        .run_unscoped(&format!(
            "DEFINE NAMESPACE IF NOT EXISTS {namespace} REPLICATION NONE;"
        ))
        .await
        .expect_err("a statement about the whole store is not its to make");
    assert!(store.to_string().contains("whole store"), "{store}");
    let own = server
        .run(
            "SELECT count(*) AS n FROM buckets GROUP BY region;",
            Vec::new(),
        )
        .await
        .expect("its own database answers");
    assert!(!own.is_empty());
}
