//! The first node of a deployment whose account may declare the namespace — the store's owner, the way a node has
//! always been able to start on an empty store — reads its stored settings before anything is declared: the namespace
//! and the database are declared first, nothing is stored yet, the environment's values are kept, and the next start
//! reads them back from the store.

use tessari_s3_infrastructure::tessaridb::MetaPool;
use tessari_s3_storage::settings::SettingsService;

use crate::{IGNORED, SCHEMA_TURN, live_config, scratch_dir};

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_first_start_on_an_undeclared_namespace_keeps_the_environment_and_reads_it_back() {
    let _why = IGNORED;
    let config = live_config(crate::test_node(), None, &scratch_dir());
    let settings = SettingsService::new(MetaPool::new(config.meta.clone()).expect("pool"));
    let env = |key: &str| (key == "TESSARIDB_S3_RECLAIM_INTERVAL_SECS").then(|| "75".to_owned());
    let _turn = SCHEMA_TURN.lock().await;
    let first = settings
        .resolve(&config.region, &env, None)
        .await
        .expect("a namespace nobody declared yet holds no settings");
    assert_eq!(
        first
            .lookup(&env, "TESSARIDB_S3_RECLAIM_INTERVAL_SECS")
            .as_deref(),
        Some("75")
    );
    settings
        .keep(&config.region, &first, None)
        .await
        .expect("kept");
    let none = |_: &str| None;
    let again = settings
        .resolve(&config.region, &none, None)
        .await
        .expect("read back");
    assert_eq!(
        again
            .lookup(&none, "TESSARIDB_S3_RECLAIM_INTERVAL_SECS")
            .as_deref(),
        Some("75")
    );
}
