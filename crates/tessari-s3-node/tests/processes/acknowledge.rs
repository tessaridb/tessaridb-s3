//! The region-level acknowledgement on a metadata store that cannot count one: three TessariDB nodes whose rows name
//! no region refuse a region majority, and the bucket keeps the majority with the store's reason in hand.

use tessari_s3_storage::Storage;
use tessari_s3_storage::buckets::{Acknowledgement, AcknowledgementSet};
use tessari_s3_types::BucketName;

use crate::faults::require_docker;
use crate::meta::MetaCluster;
use crate::requests::call;
use crate::s3::{Cluster, scratch};

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Docker and the tessaridb/tessaridb:0.36.2 image"]
async fn a_region_level_the_store_cannot_count_is_refused_in_its_words() {
    let _quiet = crate::QUIET.lock().await;
    require_docker();
    let metadata = MetaCluster::start().await;
    let cluster = Cluster::start(metadata.meta()).await;
    assert_eq!(call(&cluster.nodes[0], "PUT", "/acked", b"").await.0, 200);
    let storage = Storage::new(cluster.meta.clone(), "us-east-1", scratch("acknowledge"))
        .expect("storage over the cluster's pool");
    let name = BucketName::new("acked").expect("a name");
    let set = storage
        .buckets()
        .set_acknowledgement(&name, Acknowledgement::RegionMajority)
        .await
        .expect("the store answers");
    match set {
        AcknowledgementSet::Refused(words) => assert!(
            words.contains("region") || words.contains("MAJORITY"),
            "the store's words: {words}"
        ),
        other => panic!("a region majority this store cannot count was {other:?}"),
    }
    assert_eq!(
        call(&cluster.nodes[0], "PUT", "/acked/k", b"one").await.0,
        200,
        "the bucket kept the majority"
    );
    // Marked by hand, the bucket's object writes commit asking for the region's majority, which this store refuses:
    // the write is not acknowledged and nothing of it is kept.
    cluster
        .meta
        .run(
            "UPDATE buckets:'acked' SET acknowledge = 'local';",
            Vec::new(),
        )
        .await
        .expect("marked");
    let put = call(&cluster.nodes[0], "PUT", "/acked/k2", b"two").await;
    assert_ne!(
        put.0, 200,
        "a write the store cannot count was acknowledged"
    );
    assert_eq!(
        call(&cluster.nodes[0], "GET", "/acked/k2", b"").await.0,
        404,
        "nothing of it is kept"
    );
}
