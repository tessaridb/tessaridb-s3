//! Block Public Access against a real node (S3 ref 04 M5): every switch on for a bucket that stored none, a public
//! policy refused while BlockPublicPolicy is on, a public policy's reach kept inside the bucket's space while
//! RestrictPublicBuckets is on, and a removed configuration blocking nothing.

use tessari_s3_constants::DEFAULT_SPACE;
use tessari_s3_core::authz::Role;

use crate::access::user_with_key;
use crate::policy::{public_read, put_policy};
use crate::policy_enforced::anonymous;
use crate::{IGNORED, Seen, call, call_as, fresh};

fn configuration(switches: [bool; 4]) -> String {
    let [acls, ignore, policy, restrict] = switches;
    format!(
        r#"<PublicAccessBlockConfiguration xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><BlockPublicAcls>{acls}</BlockPublicAcls><IgnorePublicAcls>{ignore}</IgnorePublicAcls><BlockPublicPolicy>{policy}</BlockPublicPolicy><RestrictPublicBuckets>{restrict}</RestrictPublicBuckets></PublicAccessBlockConfiguration>"#
    )
}

pub(crate) async fn put_block(
    state: &tessari_s3_api::ApiState,
    bucket: &str,
    switches: [bool; 4],
) -> Seen {
    call(
        state,
        "PUT",
        &format!("/{bucket}"),
        vec![("publicAccessBlock", None)],
        configuration(switches).as_bytes(),
    )
    .await
}

/// Turns every switch off so a public policy may be written and binds as written.
pub(crate) async fn allow_public(state: &tessari_s3_api::ApiState, bucket: &str) {
    let seen = put_block(state, bucket, [false; 4]).await;
    assert_eq!(seen.status, 200, "{}", seen.body);
}

async fn read_block(state: &tessari_s3_api::ApiState, bucket: &str) -> Seen {
    call(
        state,
        "GET",
        &format!("/{bucket}"),
        vec![("publicAccessBlock", None)],
        b"",
    )
    .await
}

async fn is_public(state: &tessari_s3_api::ApiState, bucket: &str) -> Seen {
    call(
        state,
        "GET",
        &format!("/{bucket}"),
        vec![("policyStatus", None)],
        b"",
    )
    .await
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_bucket_with_no_configuration_blocks_public_policies() {
    let _why = IGNORED;
    let (state, _pool) = fresh().await;
    assert_eq!(call(&state, "PUT", "/held", vec![], b"").await.status, 200);
    let read = read_block(&state, "held").await;
    assert_eq!(read.status, 200, "{}", read.body);
    for switch in [
        "BlockPublicAcls",
        "IgnorePublicAcls",
        "BlockPublicPolicy",
        "RestrictPublicBuckets",
    ] {
        assert!(
            read.body.contains(&format!("<{switch}>true</{switch}>")),
            "{switch}: {}",
            read.body
        );
    }
    let refused = put_policy(&state, "held", &public_read("held")).await;
    assert_eq!(
        (refused.status, refused.code.as_deref()),
        (403, Some("AccessDenied")),
        "{}",
        refused.body
    );
    assert_eq!(
        call(&state, "GET", "/held", vec![("policy", None)], b"")
            .await
            .code
            .as_deref(),
        Some("NoSuchBucketPolicy"),
        "nothing was stored"
    );
    let named = format!(
        r#"{{"Version":"2012-10-17","Statement":[{{"Effect":"Allow","Principal":{{"AWS":"arn:aws:iam::{DEFAULT_SPACE}:root"}},"Action":"s3:GetObject","Resource":"arn:aws:s3:::held/*"}}]}}"#
    );
    assert_eq!(put_policy(&state, "held", &named).await.status, 204);
    let status = is_public(&state, "held").await;
    assert_eq!(status.status, 200, "{}", status.body);
    assert!(
        status.body.contains("<IsPublic>false</IsPublic>"),
        "{}",
        status.body
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn restricting_keeps_a_public_policy_inside_the_space_and_removing_blocks_nothing() {
    let _why = IGNORED;
    let (state, _pool) = fresh().await;
    let member = user_with_key(state.storage(), "bob", DEFAULT_SPACE, Role::Member, false).await;
    let stranger = user_with_key(state.storage(), "eve", "other", Role::SpaceAdmin, false).await;
    assert_eq!(call(&state, "PUT", "/pub", vec![], b"").await.status, 200);
    assert_eq!(
        call(&state, "PUT", "/pub/a.txt", vec![], b"hello")
            .await
            .status,
        200
    );
    assert_eq!(
        is_public(&state, "pub").await.code.as_deref(),
        Some("NoSuchBucketPolicy")
    );
    allow_public(&state, "pub").await;
    assert_eq!(
        put_policy(&state, "pub", &public_read("pub")).await.status,
        204
    );
    assert!(
        is_public(&state, "pub")
            .await
            .body
            .contains("<IsPublic>true</IsPublic>")
    );
    let read_as = |pair| call_as(&state, pair, "GET", "/pub/a.txt", vec![], vec![], b"");
    assert_eq!(
        anonymous(&state, "GET", "/pub/a.txt", b"").await.status,
        200
    );
    assert_eq!(read_as(stranger.pair()).await.status, 200);
    // Restricting does not touch the stored policy; it narrows who its public statements reach.
    assert_eq!(
        put_block(&state, "pub", [false, false, false, true])
            .await
            .status,
        200
    );
    let refused = anonymous(&state, "GET", "/pub/a.txt", b"").await;
    assert_eq!(
        (refused.status, refused.code.as_deref()),
        (403, Some("AccessDenied"))
    );
    assert_eq!(
        read_as(stranger.pair()).await.code.as_deref(),
        Some("AccessDenied")
    );
    assert_eq!(read_as(member.pair()).await.status, 200);
    // Removed, the configuration blocks nothing — and is answered as absent.
    assert_eq!(
        call(
            &state,
            "DELETE",
            "/pub",
            vec![("publicAccessBlock", None)],
            b""
        )
        .await
        .status,
        204
    );
    let absent = read_block(&state, "pub").await;
    assert_eq!(
        (absent.status, absent.code.as_deref()),
        (404, Some("NoSuchPublicAccessBlockConfiguration"))
    );
    assert_eq!(
        anonymous(&state, "GET", "/pub/a.txt", b"").await.status,
        200
    );
    let malformed = call(
        &state,
        "PUT",
        "/pub",
        vec![("publicAccessBlock", None)],
        b"<PublicAccessBlockConfiguration><BlockPublicAcls>maybe</BlockPublicAcls></PublicAccessBlockConfiguration>",
    )
    .await;
    assert_eq!(malformed.code.as_deref(), Some("MalformedXML"));
}
