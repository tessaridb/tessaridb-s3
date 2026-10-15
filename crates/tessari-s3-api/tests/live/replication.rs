//! A bucket's replication configuration: stored whole, read back, removed, and refused as S3 refuses it.

use crate::{IGNORED, call, fresh};

const ENABLED: &[u8] =
    b"<VersioningConfiguration><Status>Enabled</Status></VersioningConfiguration>";

fn configuration(destination: &str, rule_extra: &str) -> String {
    format!(
        "<ReplicationConfiguration><Role>arn:aws:iam::000000000000:role/replication</Role>\
         <Rule><ID>logs</ID><Priority>1</Priority><Status>Enabled</Status><Filter><Prefix>logs/</Prefix></Filter>\
         <DeleteMarkerReplication><Status>Disabled</Status></DeleteMarkerReplication>\
         <Destination><Bucket>arn:aws:s3:::{destination}</Bucket></Destination>{rule_extra}</Rule>\
         </ReplicationConfiguration>"
    )
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_replication_configuration_is_kept_whole_and_refused_as_s3_refuses_it() {
    let _why = IGNORED;
    let (state, _planter) = fresh().await;
    for bucket in ["source", "backup", "plain"] {
        assert_eq!(
            call(&state, "PUT", &format!("/{bucket}"), vec![], b"")
                .await
                .status,
            200
        );
    }
    let replication = || vec![("replication", None)];
    let missing = call(&state, "GET", "/source", replication(), b"").await;
    assert_eq!(
        (missing.status, missing.code.as_deref()),
        (404, Some("ReplicationConfigurationNotFoundError"))
    );
    // Versioning is required on the bucket first, then on the destination.
    let put = |destination: &str| configuration(destination, "");
    let refused = call(
        &state,
        "PUT",
        "/source",
        replication(),
        put("backup").as_bytes(),
    )
    .await;
    assert_eq!(
        (refused.status, refused.code.as_deref()),
        (400, Some("InvalidRequest")),
        "{}",
        refused.body
    );
    assert!(
        refused
            .body
            .contains("Versioning must be &apos;Enabled&apos;"),
        "{}",
        refused.body
    );
    for bucket in ["source", "backup"] {
        assert_eq!(
            call(
                &state,
                "PUT",
                &format!("/{bucket}"),
                vec![("versioning", None)],
                ENABLED
            )
            .await
            .status,
            200
        );
    }
    for (destination, words) in [
        ("plain", "must have versioning enabled"),
        ("nowhere", "must exist"),
        ("source", "cannot be the source bucket"),
    ] {
        let refused = call(
            &state,
            "PUT",
            "/source",
            replication(),
            put(destination).as_bytes(),
        )
        .await;
        assert_eq!(
            (refused.status, refused.code.as_deref()),
            (400, Some("InvalidRequest")),
            "{destination}"
        );
        assert!(
            refused.body.contains(words),
            "{destination}: {}",
            refused.body
        );
    }
    let tagged = configuration("backup", "").replace(
        "<Filter><Prefix>logs/</Prefix></Filter>",
        "<Filter><Tag><Key>k</Key><Value>v</Value></Tag></Filter>",
    );
    let refused = call(&state, "PUT", "/source", replication(), tagged.as_bytes()).await;
    assert_eq!(
        (refused.status, refused.code.as_deref()),
        (501, Some("NotImplemented"))
    );
    assert_eq!(
        missing.status,
        call(&state, "GET", "/source", replication(), b"")
            .await
            .status,
        "nothing was stored"
    );

    let stored = call(
        &state,
        "PUT",
        "/source",
        replication(),
        put("backup").as_bytes(),
    )
    .await;
    assert_eq!(stored.status, 200, "{}", stored.body);
    let read = call(&state, "GET", "/source", replication(), b"").await;
    assert_eq!(read.status, 200);
    for part in [
        "<Role>arn:aws:iam::000000000000:role/replication</Role>",
        "<ID>logs</ID>",
        "<Priority>1</Priority>",
        "<Prefix>logs/</Prefix>",
        "<Bucket>arn:aws:s3:::backup</Bucket>",
        "<DeleteMarkerReplication><Status>Disabled</Status></DeleteMarkerReplication>",
    ] {
        assert!(read.body.contains(part), "{part} in {}", read.body);
    }
    assert_eq!(
        call(&state, "DELETE", "/source", replication(), b"")
            .await
            .status,
        204
    );
    assert_eq!(
        call(&state, "GET", "/source", replication(), b"")
            .await
            .status,
        404
    );
}
