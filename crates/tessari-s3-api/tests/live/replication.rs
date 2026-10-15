//! A bucket's replication configuration: stored whole, read back, removed, and refused as S3 refuses it; a version
//! its rule covers is stamped `PENDING` and its task queued in `replication_tasks` in the version's own commit; a
//! replica keeps its source version id, once, in its place in the key's history, and is never sent on.

use tessari_s3_infrastructure::tessaridb::{Answer, MetaPool, Value};

use tessari_s3_core::authz::{Access, Role};

use crate::access::{grant, user_with_key};
use crate::{IGNORED, Seen, call, call_as, call_with, fresh, fresh_with, scratch_dir};

pub(crate) const ENABLED: &[u8] =
    b"<VersioningConfiguration><Status>Enabled</Status></VersioningConfiguration>";

pub(crate) fn configuration(destination: &str, rule_extra: &str) -> String {
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

const REPLICA: &str = "x-tessaridb-replica-version-id";

/// A version id this server issues for sequence `seq`.
pub(crate) fn sequence_id(seq: u64, nonce: u64) -> String {
    format!("{seq:016x}{nonce:016x}")
}

pub(crate) async fn replica_put(
    state: &tessari_s3_api::ApiState,
    path: &str,
    id: &str,
    bytes: &[u8],
) -> Seen {
    call_with(state, "PUT", path, vec![], vec![(REPLICA, id)], bytes).await
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_replica_keeps_its_source_version_id_once_in_its_place_and_is_never_sent_on() {
    let _why = IGNORED;
    let (state, planter, _) = fresh_with(scratch_dir()).await;
    for bucket in ["mirror", "further"] {
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
    // The destination replicates `logs/` itself: a replica arriving there must still not be sent on.
    let configured = call(
        &state,
        "PUT",
        "/mirror",
        vec![("replication", None)],
        configuration("further", "").as_bytes(),
    )
    .await;
    assert_eq!(configured.status, 200, "{}", configured.body);

    let five = sequence_id(5, 0xabcd);
    let stored = replica_put(&state, "/mirror/logs/a", &five, b"five").await;
    assert_eq!(stored.status, 200, "{}", stored.body);
    assert_eq!(header(&stored, "x-amz-version-id"), Some(five.as_str()));
    let head = call(&state, "HEAD", "/mirror/logs/a", vec![], b"").await;
    assert_eq!(
        (
            header(&head, "x-amz-version-id"),
            header(&head, "x-amz-replication-status")
        ),
        (Some(five.as_str()), Some("REPLICA"))
    );
    assert_eq!(tasks(&planter, "logs/a").await, Vec::new(), "never sent on");

    // Sent again, even with other bytes: the version is already here and is kept as it was.
    let again = replica_put(&state, "/mirror/logs/a", &five, b"later").await;
    assert_eq!(again.status, 200, "{}", again.body);
    let read = |id: String| {
        let state = &state;
        async move {
            call(
                state,
                "GET",
                "/mirror/logs/a",
                vec![("versionId", Some(id.as_str()))],
                b"",
            )
            .await
        }
    };
    assert_eq!(read(five.clone()).await.body, "five");

    // An older version lands behind the current one.
    let three = sequence_id(3, 0x1);
    assert_eq!(
        replica_put(&state, "/mirror/logs/a", &three, b"three")
            .await
            .status,
        200
    );
    assert_eq!(read(three).await.body, "three");
    let current = call(&state, "GET", "/mirror/logs/a", vec![], b"").await;
    assert_eq!(
        (current.body.as_str(), header(&current, "x-amz-version-id")),
        ("five", Some(five.as_str()))
    );

    // Another version at sequence 5, and an id this server never issues, are refused.
    let taken = replica_put(&state, "/mirror/logs/a", &sequence_id(5, 0x2), b"x").await;
    assert_eq!(
        (taken.status, taken.code.as_deref()),
        (409, Some("OperationAborted"))
    );
    let null = replica_put(&state, "/mirror/logs/a", "null", b"x").await;
    assert_eq!(
        (null.status, null.code.as_deref()),
        (400, Some("InvalidArgument"))
    );

    // A replica delete marker under its own id hides the object; a local write then follows it in the history.
    let six = sequence_id(6, 0x6);
    let marked = call_with(
        &state,
        "DELETE",
        "/mirror/logs/a",
        vec![],
        vec![(REPLICA, six.as_str())],
        b"",
    )
    .await;
    assert_eq!(marked.status, 204, "{}", marked.body);
    assert_eq!(
        (
            header(&marked, "x-amz-delete-marker"),
            header(&marked, "x-amz-version-id")
        ),
        (Some("true"), Some(six.as_str()))
    );
    assert_eq!(
        call(&state, "HEAD", "/mirror/logs/a", vec![], b"")
            .await
            .status,
        404
    );
    let local = call(&state, "PUT", "/mirror/logs/a", vec![], b"seven").await;
    assert_eq!(local.status, 200);
    assert!(
        header(&local, "x-amz-version-id").is_some_and(|id| id.starts_with("0000000000000007")),
        "{:?}",
        header(&local, "x-amz-version-id")
    );
    assert_eq!(
        tasks(&planter, "logs/a").await.len(),
        1,
        "a local write is replicated as usual"
    );

    // A bucket that replicates keeps versioning on.
    let suspended = call(
        &state,
        "PUT",
        "/mirror",
        vec![("versioning", None)],
        b"<VersioningConfiguration><Status>Suspended</Status></VersioningConfiguration>",
    )
    .await;
    assert_eq!(
        (suspended.status, suspended.code.as_deref()),
        (409, Some("InvalidBucketState"))
    );

    // Writing objects is not choosing their history.
    let storage = state.storage();
    let bob = user_with_key(storage, "bob", "default", Role::Member, false).await;
    grant(
        storage,
        "bob",
        "mirror",
        Access {
            read: true,
            write: true,
        },
    )
    .await;
    let refused = call_as(
        &state,
        bob.pair(),
        "PUT",
        "/mirror/logs/b",
        vec![],
        vec![(REPLICA, sequence_id(1, 1).as_str())],
        b"b",
    )
    .await;
    assert_eq!(
        (refused.status, refused.code.as_deref()),
        (403, Some("AccessDenied"))
    );
    let control = call_as(
        &state,
        bob.pair(),
        "PUT",
        "/mirror/logs/b",
        vec![],
        vec![],
        b"b",
    )
    .await;
    assert_eq!(control.status, 200, "control: bob writes objects");
}
