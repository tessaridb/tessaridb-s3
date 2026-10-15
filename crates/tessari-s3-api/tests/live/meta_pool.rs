//! The metadata pool against a real node, where a burst of requests meets a cold pool: every new connection signs in
//! on its first statement, the node verifies only so many passwords at once, and a connection whose sign-in was
//! refused must never be handed to the next request as if it were signed in.

use tessari_s3_infrastructure::tessaridb::{MetaError, MetaPool};
use tessari_s3_types::SecretKey;

use crate::{IGNORED, live_config, scratch_dir, test_node};

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_burst_on_a_cold_pool_signs_in_within_the_nodes_verification_ceiling() {
    let _why = IGNORED;
    let pool = MetaPool::new(live_config(test_node(), None, &scratch_dir()).meta).expect("pool");
    let mut burst = tokio::task::JoinSet::new();
    for _ in 0..64 {
        let pool = pool.clone();
        burst.spawn(async move { pool.run_unscoped("RETURN 1;").await.map(|_| ()) });
    }
    let mut refused = Vec::new();
    while let Some(done) = burst.join_next().await {
        if let Err(error) = done.expect("the request ran") {
            refused.push(error.to_string());
        }
    }
    assert!(
        refused.is_empty(),
        "{} of 64 refused: {:?}",
        refused.len(),
        refused.first()
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_connection_whose_sign_in_was_refused_is_not_used_again() {
    let _why = IGNORED;
    let mut settings = live_config(test_node(), None, &scratch_dir()).meta;
    settings.user = format!("nobody_{}", uuid::Uuid::new_v4().simple());
    settings.password = SecretKey::new("not-anybody's-password".to_owned());
    let pool = MetaPool::new(settings).expect("pool");
    let first = pool.run_unscoped("RETURN 1;").await;
    let second = pool.run_unscoped("RETURN 1;").await;
    assert!(
        matches!(&first, Err(MetaError::Refused { .. })),
        "a wrong password is refused: {first:?}"
    );
    assert_eq!(
        first.err(),
        second.err(),
        "the second request signs in again rather than running on the refused connection"
    );
}
