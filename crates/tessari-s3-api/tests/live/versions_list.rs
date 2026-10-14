//! ListObjectVersions against a real node (S3 ref 06 M5, ref 08): keys ascending, a key's versions newest first, the
//! newest marked `IsLatest`, delete markers as `DeleteMarker`, paged by key-marker and version-id-marker without
//! repeating or skipping an entry, and rolled up at a delimiter.

use crate::multipart::{text, texts};
use crate::versions_kept::{put, set_versioning, version_header};
use crate::{IGNORED, Seen, call, fresh};

/// One listed entry in document order: kind (`Version` or `DeleteMarker`), key, version id, IsLatest.
fn entries(body: &str) -> Vec<(String, String, String, String)> {
    let mut found = Vec::new();
    let mut rest = body;
    while let Some(start) = ["<Version>", "<DeleteMarker>"]
        .iter()
        .filter_map(|tag| rest.find(tag).map(|at| (at, *tag)))
        .min()
    {
        let (at, tag) = start;
        let kind = tag.trim_matches(|c| c == '<' || c == '>');
        let close = format!("</{kind}>");
        let after = &rest[at..];
        let end = after.find(&close).expect("closed");
        let element = &after[..end];
        found.push((
            kind.to_owned(),
            text(element, "Key").expect("Key"),
            text(element, "VersionId").expect("VersionId"),
            text(element, "IsLatest").expect("IsLatest"),
        ));
        rest = &after[end..];
    }
    found
}

async fn list(
    state: &tessari_s3_api::ApiState,
    bucket: &str,
    query: Vec<(&str, Option<&str>)>,
) -> Seen {
    let mut query = query;
    query.push(("versions", None));
    call(state, "GET", &format!("/{bucket}"), query, b"").await
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn versions_list_by_key_newest_first_with_markers_and_the_null_version() {
    let _why = IGNORED;
    let (state, _planter) = fresh().await;
    assert_eq!(call(&state, "PUT", "/vlist", vec![], b"").await.status, 200);
    put(&state, "/vlist/b", b"before versioning").await;
    set_versioning(&state, "vlist", "Enabled").await;
    let a1 = version_header(&put(&state, "/vlist/a", b"a1").await).expect("id");
    let a2 = version_header(&put(&state, "/vlist/a", b"a2").await).expect("id");
    let b2 = version_header(&put(&state, "/vlist/b", b"b2").await).expect("id");
    let gone = call(&state, "DELETE", "/vlist/b", vec![], b"").await;
    let marker = version_header(&gone).expect("marker id");
    let seen = list(&state, "vlist", vec![]).await;
    assert_eq!(seen.status, 200, "{}", seen.body);
    let v = |kind: &str, key: &str, id: &str, latest: bool| {
        (
            kind.to_owned(),
            key.to_owned(),
            id.to_owned(),
            latest.to_string(),
        )
    };
    assert_eq!(
        entries(&seen.body),
        vec![
            v("Version", "a", &a2, true),
            v("Version", "a", &a1, false),
            v("DeleteMarker", "b", &marker, true),
            v("Version", "b", &b2, false),
            v("Version", "b", "null", false),
        ]
    );
    assert_eq!(text(&seen.body, "IsTruncated").as_deref(), Some("false"));
    assert_eq!(text(&seen.body, "Name").as_deref(), Some("vlist"));
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn version_pages_resume_by_both_markers_without_repeating_or_skipping() {
    let _why = IGNORED;
    let (state, _planter) = fresh().await;
    assert_eq!(call(&state, "PUT", "/vpage", vec![], b"").await.status, 200);
    set_versioning(&state, "vpage", "Enabled").await;
    for key in ["k1", "k2", "k3"] {
        for round in 0..3_u8 {
            put(&state, &format!("/vpage/{key}"), &[round]).await;
        }
    }
    let whole = entries(&list(&state, "vpage", vec![]).await.body);
    assert_eq!(whole.len(), 9);
    for size in ["1", "2", "4"] {
        let mut paged = Vec::new();
        let (mut key_marker, mut id_marker) = (None::<String>, None::<String>);
        for pages in 1.. {
            // Nine entries come in at most nine pages; more means the markers resume behind themselves.
            assert!(pages <= whole.len(), "pages of {size} do not advance");
            let mut query = vec![("max-keys", Some(size))];
            if let Some(key) = &key_marker {
                query.push(("key-marker", Some(key.as_str())));
            }
            if let Some(id) = &id_marker {
                query.push(("version-id-marker", Some(id.as_str())));
            }
            let page = list(&state, "vpage", query).await;
            assert_eq!(page.status, 200, "{}", page.body);
            paged.extend(entries(&page.body));
            if text(&page.body, "IsTruncated").as_deref() != Some("true") {
                break;
            }
            key_marker = text(&page.body, "NextKeyMarker");
            id_marker = text(&page.body, "NextVersionIdMarker");
            assert!(
                key_marker.is_some() && id_marker.is_some(),
                "a truncated page names where to resume"
            );
        }
        assert_eq!(paged, whole, "pages of {size}");
    }
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn versions_roll_up_at_a_delimiter_and_a_lone_version_marker_is_refused() {
    let _why = IGNORED;
    let (state, _planter) = fresh().await;
    assert_eq!(call(&state, "PUT", "/vroll", vec![], b"").await.status, 200);
    set_versioning(&state, "vroll", "Enabled").await;
    for path in ["/vroll/dir/x", "/vroll/dir/x", "/vroll/dir/y", "/vroll/top"] {
        put(&state, path, b"v").await;
    }
    let seen = list(&state, "vroll", vec![("delimiter", Some("/"))]).await;
    assert_eq!(seen.status, 200, "{}", seen.body);
    assert_eq!(
        texts(&seen.body, "CommonPrefixes", "Prefix"),
        vec!["dir/".to_owned()]
    );
    let keys: Vec<String> = entries(&seen.body)
        .into_iter()
        .map(|(_, key, _, _)| key)
        .collect();
    assert_eq!(keys, vec!["top".to_owned()]);
    let lone = list(&state, "vroll", vec![("version-id-marker", Some("null"))]).await;
    assert_eq!(lone.status, 400, "{}", lone.body);
    assert_eq!(lone.code.as_deref(), Some("InvalidArgument"));
}
