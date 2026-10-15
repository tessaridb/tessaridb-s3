//! A node asking to join a running cluster of six real processes: one nobody admitted, and one presenting another
//! node's certificate, refuse to start and leave the topology as it was; one started with a join token issued for it
//! binds its certificate and takes its place.

use tessari_s3_storage::Storage;
use tessari_s3_storage::members::{Issued, MembershipService, TokenLifetime};
use tessari_s3_types::NodeId;

use crate::meta_from_env;
use crate::s3::{Cluster, count, scratch};

/// The cluster's membership, read and written as the console would.
fn members(cluster: &Cluster) -> MembershipService {
    Storage::new(cluster.meta.clone(), scratch("unused"))
        .members()
        .clone()
}

fn n7() -> NodeId {
    NodeId::new("n7").expect("an id")
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_node_nobody_admitted_refuses_to_start_and_registers_nothing() {
    let mut cluster = Cluster::start(meta_from_env()).await;
    let refused = cluster.join(&[]).await.expect_err("n7 is nobody's member");
    assert!(refused.contains("is not an admitted member"), "{refused}");
    assert_eq!(
        count(&cluster.meta, "s3_nodes").await,
        6,
        "the topology is as it was"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_node_presenting_another_nodes_certificate_refuses_to_start() {
    let mut cluster = Cluster::start(meta_from_env()).await;
    members(&cluster)
        .approve(&n7(), None, "root", "admitted by id")
        .await
        .expect("approved");
    let (cert, key) = (
        cluster.credential("n1-cert.pem"),
        cluster.credential("n1-key.pem"),
    );
    let refused = cluster
        .join(&[
            ("TESSARIDB_S3_INTERNAL_TLS_CERT", &cert),
            ("TESSARIDB_S3_INTERNAL_TLS_KEY", &key),
        ])
        .await
        .expect_err("n1's certificate does not speak for n7");
    assert!(
        refused.contains("TESSARIDB_S3_INTERNAL_TLS_CERT"),
        "{refused}"
    );
    assert_eq!(
        count(&cluster.meta, "s3_nodes").await,
        6,
        "the topology is as it was"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_node_started_with_its_join_token_binds_its_certificate_and_takes_its_place() {
    let mut cluster = Cluster::start(meta_from_env()).await;
    let members = members(&cluster);
    let lifetime = TokenLifetime::new(600).expect("a lifetime");
    let Issued::Issued { token, .. } = members
        .issue_token(&n7(), lifetime, "root", "joining")
        .await
        .expect("issued")
    else {
        panic!("n7 was removed");
    };
    cluster
        .join(&[("TESSARIDB_S3_JOIN_TOKEN", token.expose())])
        .await
        .expect("n7 joins with its token");
    assert_eq!(count(&cluster.meta, "s3_nodes").await, 7);
    let bound = members.get(&n7()).await.expect("read").expect("a record");
    assert!(
        bound.fingerprint.is_some() && bound.token_expires.is_none(),
        "the token was consumed and the certificate bound: {bound:?}"
    );
}
