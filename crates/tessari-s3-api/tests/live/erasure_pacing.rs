//! Pacing a move: a pass keeps to its budget of bytes a second, and a copy that outlives its claim renews it — or stops,
//! switching nothing, once another node has taken the claim.

use std::num::NonZeroU64;

use tessari_s3_infrastructure::tessaridb::Value;

use crate::IGNORED;
use crate::erasure_heals::data_id;
use crate::erasure_moves::{NEW_SET, applied, grown, layout_of};
use crate::erasure_reads::{read_back, striped, written};

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_move_keeps_to_its_budget_of_bytes_a_second() {
    let _why = IGNORED;
    let mut member = written().await;
    grown(&mut member).await;
    applied(&member, &[&NEW_SET]).await;
    // Six shards of three 256 KiB blocks move: 4.5 MiB, which 2 MiB a second cannot copy in under 2.25 s.
    let budget = NonZeroU64::new(2 * 1024 * 1024).expect("a budget");
    let started = std::time::Instant::now();
    let pass = member
        .storage
        .objects()
        .move_layouts(0, 100, budget)
        .await
        .expect("a pass");
    assert_eq!(pass.moved, 1);
    assert!(
        started.elapsed() >= std::time::Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(read_back(&member).await, striped());
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_long_copy_whose_claim_another_node_takes_stops_and_switches_nothing() {
    let _why = IGNORED;
    let mut member = written().await;
    grown(&mut member).await;
    applied(&member, &[&NEW_SET]).await;
    let id = data_id(&member).await;
    // 4.5 MiB at 64 KiB a second: a copy of about 72 s, outliving the first renewal of its claim at 40 s.
    let budget = NonZeroU64::new(64 * 1024).expect("a budget");
    let started = std::time::Instant::now();
    let pass = member.storage.objects().move_layouts(0, 100, budget);
    let steal = async {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        member
            .planter
            .run(
                "SET heal_claims:$data = 'n9' EXPIRE 600s;",
                vec![("data".to_owned(), Value::Uuid(id))],
            )
            .await
            .expect("taken over");
    };
    let (pass, ()) = tokio::join!(pass, steal);
    assert_eq!(pass.expect("a pass").moved, 0);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(70),
        "stopped at the renewal"
    );
    assert_eq!(layout_of(&member).await, 1, "nothing switched");
}
