//! Replication of sealed versions (G017 W3c): the replicator opens a sealed source under its own data key, and the
//! replica asks the destination to seal it, so the bytes are sealed on both sides; a plain version stays plain.

use tessari_s3_daemons::{Daemon, Replicator};
use tessari_s3_infrastructure::ReplicaClient;

use crate::replication_send::replicating;
use crate::sse::{SSE, body, drive_holds, sse_header, unseal};
use crate::{IGNORED, call, call_with, fresh_with, scratch_dir};

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_sealed_version_is_replicated_sealed_and_a_plain_one_plain() {
    let _why = IGNORED;
    let (state, _planter, dir) = fresh_with(scratch_dir()).await;
    replicating(&state).await;
    unseal(&state).await;
    let bytes = body();
    let needle = &bytes[..32];
    let sealed = call_with(
        &state,
        "PUT",
        "/origin/logs/sealed",
        vec![],
        vec![SSE],
        &bytes,
    )
    .await;
    assert_eq!(sealed.status, 200, "{}", sealed.body);
    let plain = call(&state, "PUT", "/origin/logs/plain", vec![], &bytes).await;
    assert_eq!(plain.status, 200, "{}", plain.body);

    let replicator = Replicator::new(
        state.storage().clone(),
        ReplicaClient::new().expect("a client"),
    );
    let report = replicator.run_once().await.expect("a run");
    assert_eq!(report.message, "replicated 2, to retry 0");

    let replica = call(&state, "GET", "/copies/logs/sealed", vec![], b"").await;
    assert_eq!(replica.status, 200, "{}", replica.body);
    assert_eq!(replica.bytes, bytes, "the replica reads back the plaintext");
    assert_eq!(sse_header(&replica), Some(&b"AES256"[..]));
    assert!(
        !drive_holds(&state, &dir, "copies", "logs/sealed", needle).await,
        "the replica is plaintext on the drive"
    );
    let copy = call(&state, "GET", "/copies/logs/plain", vec![], b"").await;
    assert_eq!(copy.bytes, bytes);
    assert_eq!(sse_header(&copy), None, "a plain version stays plain");
    assert!(
        drive_holds(&state, &dir, "copies", "logs/plain", needle).await,
        "control: the scan finds a plain replica"
    );
    std::fs::remove_dir_all(dir).expect("cleanup");
}
