//! Reclaimers of several nodes taking from one `reclaims` queue while writers queue the data they replace: a claim
//! that collides is answered `retry` by the store, and the reclaimer asks again rather than ending its pass.

use std::time::{Duration, Instant};

use futures_util::future::join_all;
use tessari_s3_api::ApiState;

use crate::large::body;
use crate::{call, live_config, prepared, scratch_dir, test_node};

/// Reclaim passes run at once, one per node of a six-node cluster.
const PASSES: usize = 6;

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn reclaim_passes_beside_overwriting_writers_all_succeed() {
    let dir = scratch_dir();
    // Writes replaced by later ones become reclaimable after 2 s, past any upload here, as in a deployment.
    let mut config = live_config(test_node(), None, &dir);
    config.reclaim_grace_secs = 2;
    let state = ApiState::new(&config, ApiState::system_clock(), prepared(&config).await);
    assert_eq!(call(&state, "PUT", "/churn", vec![], b"").await.status, 200);
    let until = Instant::now()
        .checked_add(Duration::from_secs(12))
        .expect("in range");
    let writers = join_all((0..4_u8).map(|writer| {
        let state = state.clone();
        async move {
            let mut turn = 0_u8;
            while Instant::now() < until {
                let path = format!("/churn/w{writer}-{}", turn.checked_rem(4).unwrap_or(0));
                let status = call(&state, "PUT", &path, vec![], &body(4096, turn))
                    .await
                    .status;
                assert_eq!(status, 200, "{path}");
                turn = turn.wrapping_add(1);
            }
        }
    }));
    let objects = state.storage().objects();
    let reclaimers = join_all((0..PASSES).map(|_| {
        let objects = objects.clone();
        async move {
            let mut failed = Vec::new();
            while Instant::now() < until {
                if let Err(error) = objects.reclaim(0, 1000).await {
                    failed.push(error.to_string());
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            failed
        }
    }));
    let ((), failed) = tokio::join!(
        async {
            writers.await;
        },
        reclaimers
    );
    let failed: Vec<String> = failed.into_iter().flatten().collect();
    assert!(
        failed.is_empty(),
        "{} passes failed: {:?}",
        failed.len(),
        failed.first()
    );
    std::fs::remove_dir_all(&dir).expect("cleanup");
}
