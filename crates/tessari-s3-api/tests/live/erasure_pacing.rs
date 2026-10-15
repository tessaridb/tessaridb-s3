//! Pacing a move: a pass keeps to its budget of bytes a second, and a copy that outlives its claim renews it — or stops,
//! switching nothing, once another node has taken the claim.

use std::num::NonZeroU64;

use std::time::{Duration, Instant};

use tessari_s3_infrastructure::tessaridb::{Answer, MetaPool, RefusalClass, Value};

use crate::IGNORED;
use crate::erasure_heals::data_id;
use crate::erasure_moves::{NEW_SET, applied, grown, layout_of};
use crate::erasure_reads::{read_back, striped, written};

/// Waits until the mover holds the claim on data `id`: a readiness signal polled to a deadline, not a timing assertion.
async fn held_by_this_node(planter: &MetaPool, id: [u8; 16]) {
    let started = Instant::now();
    loop {
        let answers = planter
            .run(
                "GET heal_claims:$data;",
                vec![("data".to_owned(), Value::Uuid(id))],
            )
            .await
            .expect("read");
        if let Some(Answer::Value {
            value: Value::String(holder),
            ..
        }) = answers.first()
            && holder == "n1"
        {
            return;
        }
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "the mover never took the claim"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Another node (`n9`) takes the claim on data `id` over, asking again while the store answers `retry`, as a careful
/// caller does.
async fn take_over(planter: &MetaPool, id: [u8; 16]) {
    for _ in 0..8 {
        match planter
            .run(
                "SET heal_claims:$data = 'n9' EXPIRE 600s;",
                vec![("data".to_owned(), Value::Uuid(id))],
            )
            .await
        {
            Ok(_) => return,
            Err(error) if error.is_class(RefusalClass::Retry) => {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(error) => panic!("taken over: {error}"),
        }
    }
    panic!("the claim could not be taken over");
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_move_keeps_to_its_budget_of_bytes_a_second() {
    let _why = IGNORED;
    let mut member = written().await;
    grown(&mut member).await;
    applied(&member, &[&NEW_SET]).await;
    // Six shards of three 256 KiB blocks move: 4.5 MiB, which 2 MiB a second cannot copy in under 2.25 s.
    let budget = NonZeroU64::new(2 * 1024 * 1024).expect("a budget");
    let started = Instant::now();
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
    let started = Instant::now();
    let pass = member.storage.objects().move_layouts(0, 100, budget);
    let steal = async {
        held_by_this_node(&member.planter, id).await;
        take_over(&member.planter, id).await;
    };
    let (pass, ()) = tokio::join!(pass, steal);
    assert_eq!(pass.expect("a pass").moved, 0);
    assert!(
        started.elapsed() < Duration::from_secs(70),
        "stopped at the renewal"
    );
    assert_eq!(layout_of(&member).await, 1, "nothing switched");
}
