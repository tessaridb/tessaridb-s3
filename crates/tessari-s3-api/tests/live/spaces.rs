//! Buckets and the spaces that own them, against a real node: a listing scoped to a space returns that space's
//! buckets only, and a bucket written before spaces existed is moved into the default space when the schema runs.

use tessari_s3_core::authz::{SpaceName, Visible};
use tessari_s3_infrastructure::tessaridb::Value;
use tessari_s3_types::BucketName;

use tessari_s3_infrastructure::tessaridb::MetaPool;

use crate::{IGNORED, SCHEMA_TURN, call, fresh, live_config, prepared, test_node};

fn name(text: &str) -> BucketName {
    BucketName::new(text).expect("a bucket name")
}

fn space(text: &str) -> SpaceName {
    SpaceName::new(text).expect("a space name")
}

fn names(buckets: &[tessari_s3_storage::buckets::Bucket]) -> Vec<&str> {
    buckets.iter().map(|bucket| bucket.name.as_str()).collect()
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_listing_scoped_to_a_space_returns_only_its_buckets() {
    let _why = IGNORED;
    let (state, _) = fresh().await;
    let buckets = state.storage().buckets();
    let region = state.region();
    buckets
        .create(&name("alpha-photos"), region, &space("alpha"), Some("ann"))
        .await
        .expect("created");
    buckets
        .create(&name("beta-photos"), region, &space("beta"), None)
        .await
        .expect("created");
    assert_eq!(
        call(&state, "PUT", "/rooted", vec![], b"").await.status,
        200
    );

    let alpha = buckets
        .list(&Visible::Space(space("alpha")))
        .await
        .expect("listed");
    assert_eq!(names(&alpha), vec!["alpha-photos"]);
    assert_eq!(alpha[0].creator.as_deref(), Some("ann"));
    assert_eq!(
        names(
            &buckets
                .list(&Visible::Space(SpaceName::default_space()))
                .await
                .expect("listed")
        ),
        vec!["rooted"],
        "the root credential's bucket is in the default space"
    );
    assert_eq!(
        names(&buckets.list(&Visible::All).await.expect("listed")),
        vec!["alpha-photos", "beta-photos", "rooted"]
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_bucket_from_before_spaces_moves_into_the_default_space() {
    let _why = IGNORED;
    // A namespace as an earlier release left it: the bucket table without `space_name`, and one bucket in it. Once the
    // current schema declares the field, its default fills every new write, so the record must exist first.
    let config = live_config(test_node(), None, None);
    let pool = MetaPool::new(config.meta.clone()).expect("pool");
    let (namespace, database) = pool.scope();
    let replication = pool.replication().clause();
    // Catalog definitions contend store-wide (Q-S3-2); this setup takes the same turn the other tests' schemas take.
    let turn = SCHEMA_TURN.lock().await;
    pool.run_unscoped(&format!(
        "DEFINE NAMESPACE IF NOT EXISTS {namespace} {replication}; USE NAMESPACE {namespace};\n\
         DEFINE DATABASE IF NOT EXISTS {database};"
    ))
    .await
    .expect("tenancy");
    let earlier = "DEFINE TABLE IF NOT EXISTS buckets (name string REQUIRED, created datetime REQUIRED, \
                   region string REQUIRED, incarnation uuid REQUIRED);\n\
                   CREATE buckets:$name = { name: $name, created: time::now(), region: 'us-east-1', \
                   incarnation: rand::uuid() };";
    pool.run(
        earlier,
        vec![("name".to_owned(), Value::String("legacy".to_owned()))],
    )
    .await
    .expect("planted");
    drop(turn);

    let storage = prepared(&config).await;
    let legacy = storage
        .buckets()
        .get(&name("legacy"))
        .await
        .expect("read")
        .expect("still there");
    assert_eq!(legacy.space, SpaceName::default_space());
    assert_eq!(legacy.creator, None);
    let scoped = Visible::Space(SpaceName::default_space());
    assert_eq!(
        names(&storage.buckets().list(&scoped).await.expect("listed")),
        vec!["legacy"],
        "found through the space index after the fill"
    );
}
