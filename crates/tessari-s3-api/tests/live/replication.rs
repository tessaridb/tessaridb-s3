//! A bucket's replication configuration: stored whole, read back, removed, and refused as S3 refuses it; a version
//! its rule covers is stamped `PENDING` and its task queued in `replication_tasks` in the version's own commit.

use tessari_s3_infrastructure::tessaridb::{Answer, MetaPool, Value};

use crate::{IGNORED, Seen, call, fresh};

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

/// The tasks queued for `key` as `(version id, destination, marker)`, sorted.
async fn tasks(planter: &MetaPool, key: &str) -> Vec<(String, String, bool)> {
    let answers = planter
        .run(
            "SELECT version_id, destination, marker FROM replication_tasks WHERE key = $key;",
            vec![("key".to_owned(), Value::String(key.to_owned()))],
        )
        .await
        .expect("outbox read");
    let Some(Answer::Records { records, .. }) = answers.into_iter().next() else {
        return Vec::new();
    };
    let mut queued: Vec<(String, String, bool)> = records
        .into_iter()
        .map(|(_, record)| {
            let Value::Object(fields) = record else {
                panic!("a task, not {record:?}");
            };
            match (
                fields.get("version_id"),
                fields.get("destination"),
                fields.get("marker"),
            ) {
                (
                    Some(Value::String(version)),
                    Some(Value::String(destination)),
                    Some(Value::Bool(marker)),
                ) => (version.clone(), destination.clone(), *marker),
                other => panic!("a task's fields, not {other:?}"),
            }
        })
        .collect();
    queued.sort();
    queued
}

fn header<'a>(seen: &'a Seen, name: &str) -> Option<&'a str> {
    seen.headers.get(name).and_then(|value| value.to_str().ok())
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_version_its_rule_covers_is_pending_and_its_task_queued_in_the_same_commit() {
    let _why = IGNORED;
    let (state, planter) = fresh().await;
    for bucket in ["origin", "copies"] {
        assert_eq!(
            call(&state, "PUT", &format!("/{bucket}"), vec![], b"")
                .await
                .status,
            200
        );
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
    let configure = |markers: &str| {
        configuration("copies", "").replace(
            "<DeleteMarkerReplication><Status>Disabled</Status>",
            &format!("<DeleteMarkerReplication><Status>{markers}</Status>"),
        )
    };
    let replication = || vec![("replication", None)];
    let stored = call(
        &state,
        "PUT",
        "/origin",
        replication(),
        configure("Disabled").as_bytes(),
    )
    .await;
    assert_eq!(stored.status, 200, "{}", stored.body);

    let written = call(&state, "PUT", "/origin/logs/a", vec![], b"one").await;
    assert_eq!(written.status, 200, "{}", written.body);
    let version = header(&written, "x-amz-version-id")
        .expect("a version id")
        .to_owned();
    let head = call(&state, "HEAD", "/origin/logs/a", vec![], b"").await;
    assert_eq!(header(&head, "x-amz-replication-status"), Some("PENDING"));
    let by_version = call(
        &state,
        "HEAD",
        "/origin/logs/a",
        vec![("versionId", Some(&version))],
        b"",
    )
    .await;
    assert_eq!(
        header(&by_version, "x-amz-replication-status"),
        Some("PENDING")
    );
    assert_eq!(
        tasks(&planter, "logs/a").await,
        vec![(version.clone(), "copies".to_owned(), false)]
    );
    // A key the rule does not cover is neither stamped nor queued.
    assert_eq!(
        call(&state, "PUT", "/origin/other/b", vec![], b"two")
            .await
            .status,
        200
    );
    let other = call(&state, "HEAD", "/origin/other/b", vec![], b"").await;
    assert_eq!(header(&other, "x-amz-replication-status"), None);
    assert_eq!(tasks(&planter, "other/b").await, Vec::new());

    // A delete marker follows only when the rule replicates delete markers.
    assert_eq!(
        call(&state, "DELETE", "/origin/logs/a", vec![], b"")
            .await
            .status,
        204
    );
    assert_eq!(tasks(&planter, "logs/a").await.len(), 1);
    assert_eq!(
        call(
            &state,
            "PUT",
            "/origin",
            replication(),
            configure("Enabled").as_bytes()
        )
        .await
        .status,
        200
    );
    let marked = call(&state, "DELETE", "/origin/logs/a", vec![], b"").await;
    assert_eq!(marked.status, 204);
    let marker = header(&marked, "x-amz-version-id")
        .expect("a marker id")
        .to_owned();
    let queued = tasks(&planter, "logs/a").await;
    assert_eq!(queued.len(), 2, "{queued:?}");
    assert!(
        queued.contains(&(marker, "copies".to_owned(), true)),
        "{queued:?}"
    );
}
