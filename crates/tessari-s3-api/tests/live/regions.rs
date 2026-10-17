//! Two nodes of two regions over one namespace: buckets are shared, each region keeps its buckets' object metadata in
//! its own database, and a request for another region's bucket is sent to that region rather than answered from a
//! database that does not hold it.

use tessari_s3_api::{ApiState, ConsoleState};
use tessari_s3_infrastructure::tessaridb::MetaPool;

use crate::console::{send, sign_in};
use crate::erasure::count;
use crate::{IGNORED, call, live_config, prepared, scratch_dir, test_node};

const HOME: &str = "us-east-1";
const AWAY: &str = "eu-west-1";

/// A node of [`HOME`] and a node of [`AWAY`] over one fresh namespace, and the owner's handles on the global
/// database and on each region's.
async fn two_regions() -> (
    ApiState,
    ApiState,
    MetaPool,
    MetaPool,
    MetaPool,
    ConsoleState,
) {
    let home = live_config(test_node(), None, &scratch_dir());
    let mut away = home.clone();
    away.region = AWAY.to_owned();
    away.data_dir = scratch_dir();
    let home_state = ApiState::new(&home, ApiState::system_clock(), prepared(&home).await);
    let away_storage = prepared(&away).await;
    let away_console = ConsoleState::new(&away, ApiState::system_clock(), away_storage.clone())
        .expect("built")
        .expect("a console is configured");
    let away_state = ApiState::new(&away, ApiState::system_clock(), away_storage);
    let global = MetaPool::new(home.meta.clone()).expect("pool");
    let home_db = global.regional(HOME).expect("home region");
    let away_db = global.regional(AWAY).expect("away region");
    (
        home_state,
        away_state,
        global,
        home_db,
        away_db,
        away_console,
    )
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_regions_objects_live_in_its_own_database_and_its_buckets_in_the_global_one() {
    let _why = IGNORED;
    let (home, _away, global, home_db, away_db, _console) = two_regions().await;
    assert_eq!(
        call(&home, "PUT", "/shared-name", vec![], b"").await.status,
        200
    );
    let put = call(&home, "PUT", "/shared-name/k", vec![], b"bytes").await;
    assert_eq!(put.status, 200, "{}", put.body);
    let answers = global
        .run("SELECT region FROM ONLY buckets:'shared-name';", Vec::new())
        .await
        .expect("the global database holds the bucket");
    assert!(
        format!("{answers:?}").contains(HOME),
        "the bucket names its region: {answers:?}"
    );
    assert_eq!(
        count(&home_db, "objects").await,
        1,
        "the home region holds the object"
    );
    assert_eq!(
        count(&away_db, "objects").await,
        0,
        "the other region holds none of it"
    );
    assert!(
        global
            .run("SELECT count() FROM objects GROUP ALL;", Vec::new())
            .await
            .is_err(),
        "the global database has no object table"
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_request_for_another_regions_bucket_is_sent_to_that_region_and_writes_nothing() {
    let _why = IGNORED;
    let (home, away, _global, home_db, away_db, away_console) = two_regions().await;
    assert_eq!(call(&home, "PUT", "/homed", vec![], b"").await.status, 200);
    assert_eq!(
        call(&home, "PUT", "/homed/k", vec![], b"one").await.status,
        200
    );
    for (method, path, query, body) in [
        ("PUT", "/homed/k2", vec![], &b"two"[..]),
        ("GET", "/homed/k", vec![], &b""[..]),
        ("HEAD", "/homed/k", vec![], &b""[..]),
        ("DELETE", "/homed/k", vec![], &b""[..]),
        ("GET", "/homed", vec![("list-type", Some("2"))], &b""[..]),
        ("GET", "/homed", vec![("versions", None)], &b""[..]),
        ("GET", "/homed", vec![("uploads", None)], &b""[..]),
        ("POST", "/homed/m", vec![("uploads", None)], &b""[..]),
        ("HEAD", "/homed", vec![], &b""[..]),
        ("GET", "/homed", vec![("versioning", None)], &b""[..]),
        (
            "PUT",
            "/homed",
            vec![("versioning", None)],
            &b"<VersioningConfiguration><Status>Enabled</Status></VersioningConfiguration>"[..],
        ),
        ("DELETE", "/homed", vec![], &b""[..]),
    ] {
        let seen = call(&away, method, path, query, body).await;
        assert_eq!(seen.status, 301, "{method} {path}: {}", seen.body);
        assert_eq!(
            seen.headers
                .get("x-amz-bucket-region")
                .and_then(|v| v.to_str().ok()),
            Some(HOME),
            "{method} {path}"
        );
        if method != "HEAD" {
            assert_eq!(
                seen.code.as_deref(),
                Some("PermanentRedirect"),
                "{method} {path}"
            );
        }
    }
    // A batch delete, sent whole as a client sends it (its body's digest included), is sent there too.
    let batch = crate::delete_many::delete_many(
        &away,
        "homed",
        "<Delete><Object><Key>k</Key></Object></Delete>",
    )
    .await;
    assert_eq!(
        (batch.status, batch.code.as_deref()),
        (301, Some("PermanentRedirect")),
        "{}",
        batch.body
    );
    assert_eq!(
        batch
            .headers
            .get("x-amz-bucket-region")
            .and_then(|v| v.to_str().ok()),
        Some(HOME)
    );
    // The other region's console refuses the bucket's objects rather than listing that region's empty database.
    let token = sign_in(&away_console).await;
    let listed = send(
        &away_console,
        "GET",
        "/api/v1/buckets/homed/objects?limit=10",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(
        (listed.status, listed.code().as_deref()),
        (409, Some("other_region"))
    );
    // Where the bucket lives is answered anywhere; that is how a client finds the region to go to.
    let location = call(&away, "GET", "/homed", vec![("location", None)], b"").await;
    assert_eq!(location.status, 200, "{}", location.body);
    // Nothing was written by the refused requests, in either region, and the bucket still stands.
    assert_eq!(count(&home_db, "objects").await, 1);
    assert_eq!(count(&away_db, "objects").await, 0);
    assert_eq!(count(&away_db, "uploads").await, 0);
    assert_eq!(call(&home, "HEAD", "/homed", vec![], b"").await.status, 200);
    assert_eq!(
        call(&home, "GET", "/homed/k", vec![], b"").await.body,
        "one"
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn the_region_level_is_proved_before_it_is_set_and_then_carried_by_every_object_write() {
    let _why = IGNORED;
    let (_home, away, global, _home_db, away_db, away_console) = two_regions().await;
    assert_eq!(call(&away, "PUT", "/acked", vec![], b"").await.status, 200);
    let token = sign_in(&away_console).await;
    // The test node is a lone store with no other voter, so it meets a region majority as it meets a majority: the
    // probe passes and the bucket takes the level. A store that cannot count one refuses it, and the bucket's writes
    // then refuse too; three real nodes prove that (`tessari-s3-node` processes, `acknowledge`).
    let set = send(
        &away_console,
        "PUT",
        "/api/v1/buckets/acked/acknowledgement",
        Some(&token),
        Some(r#"{"level":"region","reason":"live test"}"#),
    )
    .await;
    assert_eq!(set.status, 204);
    let level = |answers: &str| answers.contains("'local'") || answers.contains("\"local\"");
    let kept = global
        .run("SELECT acknowledge FROM ONLY buckets:'acked';", Vec::new())
        .await
        .expect("read");
    assert!(
        level(&format!("{kept:?}")),
        "the bucket took the level: {kept:?}"
    );
    assert_eq!(
        call(&away, "PUT", "/acked/k", vec![], b"one").await.status,
        200
    );
    let majority = send(
        &away_console,
        "PUT",
        "/api/v1/buckets/acked/acknowledgement",
        Some(&token),
        Some(r#"{"level":"majority","reason":"live test"}"#),
    )
    .await;
    assert_eq!(majority.status, 204);
    let back = global
        .run("SELECT acknowledge FROM ONLY buckets:'acked';", Vec::new())
        .await
        .expect("read");
    assert!(
        !level(&format!("{back:?}")),
        "back at the majority: {back:?}"
    );
    assert_eq!(
        call(&away, "PUT", "/acked/k2", vec![], b"two").await.status,
        200
    );
    assert_eq!(count(&away_db, "objects").await, 2, "both writes stand");
}
