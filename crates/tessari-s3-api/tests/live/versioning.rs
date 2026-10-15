//! Bucket versioning against a real node: the state a bucket starts in, the one-way move into the versioned world,
//! the configurations refused, and who may change it.

use tessari_s3_core::authz::{Access, Role};

use crate::access::{grant, user_with_key};
use crate::multipart::text;
use crate::{IGNORED, call, call_as, fresh};

const DOCUMENT: &str = "http://s3.amazonaws.com/doc/2006-03-01/";

fn configuration(inner: &str) -> Vec<u8> {
    format!("<VersioningConfiguration xmlns=\"{DOCUMENT}\">{inner}</VersioningConfiguration>")
        .into_bytes()
}

/// The `Status` a GetBucketVersioning answers, `None` when it carries none.
async fn status(state: &tessari_s3_api::ApiState) -> Option<String> {
    let seen = call(state, "GET", "/files", vec![("versioning", None)], b"").await;
    assert_eq!(seen.status, 200, "{}", seen.body);
    assert!(
        seen.body.contains("<VersioningConfiguration"),
        "{}",
        seen.body
    );
    text(&seen.body, "Status")
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn versioning_moves_into_the_versioned_world_and_never_back() {
    let _why = IGNORED;
    let (state, _) = fresh().await;
    assert_eq!(call(&state, "PUT", "/files", vec![], b"").await.status, 200);
    assert_eq!(
        status(&state).await,
        None,
        "a new bucket was never versioned"
    );

    let put = |inner: &'static str| {
        let state = state.clone();
        async move {
            call(
                &state,
                "PUT",
                "/files",
                vec![("versioning", None)],
                &configuration(inner),
            )
            .await
        }
    };
    let enabled = put("<Status>Enabled</Status>").await;
    assert_eq!(enabled.status, 200, "{}", enabled.body);
    assert_eq!(status(&state).await.as_deref(), Some("Enabled"));
    assert_eq!(put("<Status>Suspended</Status>").await.status, 200);
    assert_eq!(status(&state).await.as_deref(), Some("Suspended"));
    assert_eq!(put("<Status>Enabled</Status>").await.status, 200);

    // There is no status for "never versioned"; every other value is malformed and changes nothing.
    for refused in ["", "<Status>Disabled</Status>", "<Status>enabled</Status>"] {
        let seen = put(refused).await;
        assert_eq!(
            (seen.status, seen.code.as_deref()),
            (400, Some("MalformedXML")),
            "{refused}: {}",
            seen.body
        );
    }
    let mfa = put("<Status>Enabled</Status><MfaDelete>Enabled</MfaDelete>").await;
    assert_eq!(
        (mfa.status, mfa.code.as_deref()),
        (501, Some("NotImplemented"))
    );
    assert_eq!(
        status(&state).await.as_deref(),
        Some("Enabled"),
        "no refused configuration changed the state"
    );
    let absent = call(&state, "GET", "/nowhere", vec![("versioning", None)], b"").await;
    assert_eq!(absent.code.as_deref(), Some("NoSuchBucket"));
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn only_who_administers_a_bucket_changes_its_versioning() {
    let _why = IGNORED;
    let (state, _) = fresh().await;
    let storage = state.storage();
    let ann = user_with_key(storage, "ann", "alpha", Role::Member, true).await;
    let bob = user_with_key(storage, "bob", "alpha", Role::Member, false).await;
    let made = call_as(&state, ann.pair(), "PUT", "/files", vec![], vec![], b"").await;
    assert_eq!(made.status, 200, "{}", made.body);
    grant(
        storage,
        "bob",
        "files",
        Access {
            read: true,
            write: true,
        },
    )
    .await;
    let body = configuration("<Status>Enabled</Status>");
    let query = || vec![("versioning", None)];

    let writer = call_as(&state, bob.pair(), "PUT", "/files", query(), vec![], &body).await;
    assert_eq!(
        (writer.status, writer.code.as_deref()),
        (403, Some("AccessDenied")),
        "writing objects is not administering the bucket"
    );
    let read = call_as(&state, bob.pair(), "GET", "/files", query(), vec![], b"").await;
    assert_eq!(read.status, 200, "a reader may see the configuration");
    let owner = call_as(&state, ann.pair(), "PUT", "/files", query(), vec![], &body).await;
    assert_eq!(owner.status, 200, "{}", owner.body);
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_bucket_holding_only_versions_is_not_empty() {
    let _why = IGNORED;
    let (state, _) = fresh().await;
    assert_eq!(call(&state, "PUT", "/files", vec![], b"").await.status, 200);
    let enable = configuration("<Status>Enabled</Status>");
    let enabled = call(&state, "PUT", "/files", vec![("versioning", None)], &enable).await;
    assert_eq!(enabled.status, 200, "{}", enabled.body);
    assert_eq!(
        call(&state, "PUT", "/files/k", vec![], b"one").await.status,
        200
    );
    // A delete leaves a marker in front of the version: nothing is current, two versions remain.
    assert_eq!(
        call(&state, "DELETE", "/files/k", vec![], b"").await.status,
        204
    );
    assert_eq!(
        call(&state, "GET", "/files/k", vec![], b"").await.status,
        404
    );

    let refused = call(&state, "DELETE", "/files", vec![], b"").await;
    assert_eq!(refused.status, 409, "{}", refused.body);
    assert_eq!(
        text(&refused.body, "Code").as_deref(),
        Some("BucketNotEmpty")
    );
    assert_eq!(
        call(&state, "GET", "/files", vec![("versioning", None)], b"")
            .await
            .status,
        200
    );
}
