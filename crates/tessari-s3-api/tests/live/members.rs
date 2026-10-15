//! Cluster membership on a real TessariDB node: a node is admitted by its id, by one certificate's fingerprint, or by
//! a one-time join token that binds the certificate presenting it; a removed node is a tombstone that nothing admits
//! again; and a node the current layout places shards on is not removed.

use tessari_s3_core::erasure::Code;
use tessari_s3_storage::members::{
    Admission, Approval, Fingerprint, Issued, JoinToken, Joined, MemberState, MembershipService,
    Removal, TokenLifetime,
};
use tessari_s3_types::NodeId;

use crate::{IGNORED, fresh};

fn node(id: &str) -> NodeId {
    NodeId::new(id).expect("an id")
}

fn print(digit: char) -> Fingerprint {
    Fingerprint::new(&digit.to_string().repeat(64)).expect("a fingerprint")
}

fn minute() -> TokenLifetime {
    TokenLifetime::new(60).expect("a lifetime")
}

async fn token(members: &MembershipService, id: &str) -> JoinToken {
    match members
        .issue_token(&node(id), minute(), "root", "joining")
        .await
        .expect("issued")
    {
        Issued::Issued { token, .. } => token,
        Issued::Removed => panic!("{id} was removed"),
    }
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_node_approved_by_id_is_admitted_and_an_unknown_one_is_not() {
    let _why = IGNORED;
    let (state, _) = fresh().await;
    let members = state.storage().members();
    let approved = members
        .approve(&node("n1"), None, "root", "first node")
        .await;
    assert_eq!(approved.expect("answered"), Approval::Approved);
    assert_eq!(
        members
            .admits(&node("n1"), &print('a'))
            .await
            .expect("asked"),
        Admission::Admitted
    );
    assert_eq!(
        members
            .admits(&node("n9"), &print('a'))
            .await
            .expect("asked"),
        Admission::Unknown
    );
    let listed = members.list().await.expect("listed");
    assert_eq!(listed.len(), 1);
    assert_eq!(
        (listed[0].reason.as_str(), listed[0].by.as_str()),
        ("first node", "root")
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_pinned_node_is_admitted_only_with_its_own_certificate() {
    let _why = IGNORED;
    let (state, _) = fresh().await;
    let members = state.storage().members();
    members
        .approve(&node("n1"), Some(&print('a')), "root", "pinned")
        .await
        .expect("approved");
    assert_eq!(
        members
            .admits(&node("n1"), &print('a'))
            .await
            .expect("asked"),
        Admission::Admitted
    );
    assert_eq!(
        members
            .admits(&node("n1"), &print('b'))
            .await
            .expect("asked"),
        Admission::OtherCertificate
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_join_token_binds_the_certificate_that_offers_it_once() {
    let _why = IGNORED;
    let (state, _) = fresh().await;
    let members = state.storage().members();
    let issued = token(members, "n1").await;
    assert_eq!(
        members
            .admits(&node("n1"), &print('a'))
            .await
            .expect("asked"),
        Admission::AwaitingToken
    );
    let wrong = JoinToken::new(&"0".repeat(64)).expect("a token's shape");
    let refused = members
        .join(&node("n1"), &wrong, &print('a'))
        .await
        .expect("answered");
    assert_eq!(refused, Joined::Refused);
    // n2 has no token; n1's does not open it.
    assert_eq!(
        members
            .join(&node("n2"), &issued, &print('a'))
            .await
            .expect("answered"),
        Joined::Refused
    );
    assert_eq!(
        members
            .join(&node("n1"), &issued, &print('a'))
            .await
            .expect("answered"),
        Joined::Joined
    );
    assert_eq!(
        members
            .admits(&node("n1"), &print('a'))
            .await
            .expect("asked"),
        Admission::Admitted
    );
    assert_eq!(
        members
            .admits(&node("n1"), &print('b'))
            .await
            .expect("asked"),
        Admission::OtherCertificate
    );
    // Used once: a second offer, with another certificate, binds nothing.
    assert_eq!(
        members
            .join(&node("n1"), &issued, &print('b'))
            .await
            .expect("answered"),
        Joined::Refused
    );
    assert_eq!(
        members
            .admits(&node("n1"), &print('a'))
            .await
            .expect("asked"),
        Admission::Admitted
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_lapsed_join_token_binds_nothing() {
    let _why = IGNORED;
    let (state, planter) = fresh().await;
    let members = state.storage().members();
    let issued = token(members, "n1").await;
    planter
        .run(
            "UPDATE s3_members:'n1' SET token_expires = time::now() - duration::from_secs(1);",
            Vec::new(),
        )
        .await
        .expect("lapsed by hand");
    assert_eq!(
        members
            .join(&node("n1"), &issued, &print('a'))
            .await
            .expect("answered"),
        Joined::Refused
    );
    assert_eq!(
        members
            .admits(&node("n1"), &print('a'))
            .await
            .expect("asked"),
        Admission::AwaitingToken
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_removed_node_is_a_tombstone_nothing_admits_again() {
    let _why = IGNORED;
    let (state, _) = fresh().await;
    let members = state.storage().members();
    let cluster = state.storage().cluster();
    members
        .approve(&node("n1"), None, "root", "first")
        .await
        .expect("approved");
    cluster
        .register(&node("n1"), "x:1", None)
        .await
        .expect("registered");
    let removed = members.remove(&node("n1"), "root", "retired", None).await;
    assert_eq!(removed.expect("answered"), Removal::Removed);
    let tombstone = members.get(&node("n1")).await.expect("read").expect("kept");
    assert_eq!(
        (tombstone.state, tombstone.reason.as_str()),
        (MemberState::Removed, "retired")
    );
    assert!(
        cluster.members().await.expect("listed").is_empty(),
        "the registration survived"
    );
    assert_eq!(
        members
            .admits(&node("n1"), &print('a'))
            .await
            .expect("asked"),
        Admission::Removed
    );
    let again = members
        .approve(&node("n1"), None, "root", "back")
        .await
        .expect("answered");
    assert_eq!(again, Approval::Removed);
    let issued = members
        .issue_token(&node("n1"), minute(), "root", "back")
        .await;
    assert!(matches!(issued.expect("answered"), Issued::Removed));
    let twice = members.remove(&node("n1"), "root", "again", None).await;
    assert_eq!(twice.expect("answered"), Removal::AlreadyRemoved);
    let unknown = members.remove(&node("n9"), "root", "never", None).await;
    assert_eq!(unknown.expect("answered"), Removal::Unknown);
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_node_the_current_layout_places_shards_on_is_not_removed() {
    let _why = IGNORED;
    let (state, _) = fresh().await;
    let members = state.storage().members();
    let cluster = state.storage().cluster();
    for id in ["n1", "n2", "n3"] {
        members
            .approve(&node(id), None, "root", "founding")
            .await
            .expect("approved");
        cluster
            .register(&node(id), "x:1", None)
            .await
            .expect("registered");
    }
    let code = Code::new(2, 1).expect("a code");
    let layout = cluster
        .current(code)
        .await
        .expect("asked")
        .expect("a first layout");
    let refused = members
        .remove(&node("n2"), "root", "retire", Some(&layout))
        .await;
    assert_eq!(refused.expect("answered"), Removal::InLayout);
    let kept = members.get(&node("n2")).await.expect("read").expect("kept");
    assert_eq!(kept.state, MemberState::Approved);
}
