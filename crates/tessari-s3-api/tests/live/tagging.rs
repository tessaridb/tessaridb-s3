//! Tags against a real node (S3 ref 05 M8): an object version's tags from its write, PutObjectTagging and
//! DeleteObjectTagging (a version named by id included), each limit refused as `InvalidTag`. How tags travel with a
//! copy, a multipart upload and a bucket is `tagging_carried`.

use crate::delete_many::md5_of;
use crate::object_lock::refused;
use crate::versions_kept::{set_versioning, version_header};
use crate::{IGNORED, Seen, call, call_with, fresh};

/// A `Tagging` document holding `pairs`.
pub(crate) fn document(pairs: &[(&str, &str)]) -> String {
    let tags: String = pairs
        .iter()
        .map(|(key, value)| format!("<Tag><Key>{key}</Key><Value>{value}</Value></Tag>"))
        .collect();
    format!("<Tagging><TagSet>{tags}</TagSet></Tagging>")
}

/// The (key, value) pairs a tagging answer lists, in order.
pub(crate) fn listed(seen: &Seen) -> Vec<(String, String)> {
    seen.body
        .split("<Tag>")
        .skip(1)
        .map(|tag| {
            let between = |open: &str, close: &str| {
                tag.split(open)
                    .nth(1)
                    .and_then(|rest| rest.split(close).next())
                    .unwrap_or_default()
                    .to_owned()
            };
            (between("<Key>", "</Key>"), between("<Value>", "</Value>"))
        })
        .collect()
}

pub(crate) fn owned(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) async fn put_tags(
    state: &tessari_s3_api::ApiState,
    path: &str,
    query: Vec<(&str, Option<&str>)>,
    body: &str,
) -> Seen {
    let md5 = md5_of(body);
    let mut query = query;
    query.push(("tagging", None));
    call_with(
        state,
        "PUT",
        path,
        query,
        vec![("content-md5", md5.as_str())],
        body.as_bytes(),
    )
    .await
}

pub(crate) async fn get_tags(
    state: &tessari_s3_api::ApiState,
    path: &str,
    query: Vec<(&str, Option<&str>)>,
) -> Seen {
    let mut query = query;
    query.push(("tagging", None));
    call(state, "GET", path, query, b"").await
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn an_object_versions_tags_come_with_its_write_and_change_by_themselves() {
    let _why = IGNORED;
    let (state, _) = fresh().await;
    assert_eq!(
        call(&state, "PUT", "/tagged", vec![], b"").await.status,
        200
    );
    set_versioning(&state, "tagged", "Enabled").await;
    let first = version_header(
        &call_with(
            &state,
            "PUT",
            "/tagged/k",
            vec![],
            vec![("x-amz-tagging", "team=blue&tier=gold%20plus")],
            b"one",
        )
        .await,
    )
    .expect("an id");
    let seen = get_tags(&state, "/tagged/k", vec![]).await;
    assert_eq!(seen.status, 200, "{}", seen.body);
    assert_eq!(
        listed(&seen),
        owned(&[("team", "blue"), ("tier", "gold plus")])
    );
    let head = call(&state, "HEAD", "/tagged/k", vec![], b"").await;
    assert_eq!(
        head.headers
            .get("x-amz-tagging-count")
            .map(|v| v.as_bytes()),
        Some(&b"2"[..])
    );

    // A newer version starts untagged; the older one keeps its tags, read and changed by id.
    let second =
        version_header(&call(&state, "PUT", "/tagged/k", vec![], b"two").await).expect("an id");
    assert!(listed(&get_tags(&state, "/tagged/k", vec![]).await).is_empty());
    assert!(
        call(&state, "HEAD", "/tagged/k", vec![], b"")
            .await
            .headers
            .get("x-amz-tagging-count")
            .is_none()
    );
    let changed = put_tags(
        &state,
        "/tagged/k",
        vec![("versionId", Some(&first))],
        &document(&[("state", "old")]),
    )
    .await;
    assert_eq!(changed.status, 200, "{}", changed.body);
    assert_eq!(
        changed
            .headers
            .get("x-amz-version-id")
            .map(|v| v.as_bytes()),
        Some(first.as_bytes())
    );
    assert_eq!(
        listed(&get_tags(&state, "/tagged/k", vec![("versionId", Some(&first))]).await),
        owned(&[("state", "old")])
    );
    assert!(
        listed(&get_tags(&state, "/tagged/k", vec![("versionId", Some(&second))]).await).is_empty()
    );

    // The current version's tags, replaced then removed; its bytes never move.
    let current = put_tags(&state, "/tagged/k", vec![], &document(&[("a", "1")])).await;
    assert_eq!(current.status, 200, "{}", current.body);
    assert_eq!(
        listed(&get_tags(&state, "/tagged/k", vec![]).await),
        owned(&[("a", "1")])
    );
    // What a read of the object reports moves with it.
    let count = |seen: &Seen| {
        seen.headers
            .get("x-amz-tagging-count")
            .map(|v| v.as_bytes().to_vec())
    };
    assert_eq!(
        count(&call(&state, "HEAD", "/tagged/k", vec![], b"").await),
        Some(b"1".to_vec())
    );
    let removed = call(&state, "DELETE", "/tagged/k", vec![("tagging", None)], b"").await;
    assert_eq!(removed.status, 204, "{}", removed.body);
    assert!(listed(&get_tags(&state, "/tagged/k", vec![]).await).is_empty());
    assert_eq!(
        call(&state, "GET", "/tagged/k", vec![], b"").await.bytes,
        b"two"
    );
    assert_eq!(
        count(&call(&state, "HEAD", "/tagged/k", vec![], b"").await),
        None
    );
    // Removing the current version brings the older one back with its tags.
    let gone = call(
        &state,
        "DELETE",
        "/tagged/k",
        vec![("versionId", Some(&second))],
        b"",
    )
    .await;
    assert_eq!(gone.status, 204, "{}", gone.body);
    let back = call(&state, "GET", "/tagged/k", vec![], b"").await;
    assert_eq!(
        (back.bytes.as_slice(), count(&back)),
        (&b"one"[..], Some(b"1".to_vec()))
    );

    // Limits: eleven tags, a reserved key, a key twice — each `InvalidTag`; a missing key `NoSuchKey`.
    let eleven: Vec<(String, String)> = (0..11).map(|n| (format!("k{n}"), String::new())).collect();
    let eleven: Vec<(&str, &str)> = eleven
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    for body in [
        document(&eleven),
        document(&[("aws:x", "1")]),
        document(&[("a", "1"), ("a", "2")]),
    ] {
        assert_eq!(
            refused(&put_tags(&state, "/tagged/k", vec![], &body).await),
            (400, Some("InvalidTag")),
            "{body}"
        );
    }
    assert_eq!(
        refused(&put_tags(&state, "/tagged/absent", vec![], &document(&[("a", "1")])).await),
        (404, Some("NoSuchKey"))
    );
    let header = call_with(
        &state,
        "PUT",
        "/tagged/x",
        vec![],
        vec![("x-amz-tagging", "aws:x=1")],
        b"x",
    )
    .await;
    assert_eq!(refused(&header), (400, Some("InvalidTag")));
}
