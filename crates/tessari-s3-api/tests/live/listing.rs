//! ListObjectsV2 and ListObjects against a real node, judged by a reference model: every key under the prefix in
//! UTF-8 byte order, rolled up at the first delimiter after it, each prefix counted once.

use tessari_s3_infrastructure::tessaridb::Value;

use crate::{IGNORED, Seen, call, fresh};

/// The texts of every `<tag>` directly inside a `<parent>` element, in document order, unescaped.
fn texts(body: &str, parent: &str, tag: &str) -> Vec<String> {
    let open = format!("<{parent}><{tag}>");
    let close = format!("</{tag}>");
    body.split(&open)
        .skip(1)
        .filter_map(|rest| rest.split_once(&close).map(|(text, _)| unescape(text)))
        .collect()
}

/// The text of the first `<tag>` anywhere, unescaped.
fn text(body: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    body.split_once(&open)
        .and_then(|(_, rest)| rest.split_once(&close))
        .map(|(text, _)| unescape(text))
}

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

async fn bucket_with(state: &tessari_s3_api::ApiState, bucket: &str, keys: &[&str]) {
    assert_eq!(
        call(state, "PUT", &format!("/{bucket}"), vec![], b"")
            .await
            .status,
        200
    );
    for key in keys {
        let path = format!("/{bucket}/{}", path_encode(key));
        let put = call(state, "PUT", &path, vec![], key.as_bytes()).await;
        assert_eq!(put.status, 200, "{key}: {}", put.body);
    }
}

/// A key as a request path segment: unreserved bytes and `/` as they are, the rest percent-encoded.
fn path_encode(key: &str) -> String {
    key.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
                char::from(b).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// What S3 describes, computed directly: (keys, common prefixes).
fn reference(keys: &[&str], prefix: &str, delimiter: Option<&str>) -> (Vec<String>, Vec<String>) {
    let mut sorted: Vec<&str> = keys.to_vec();
    sorted.sort_unstable();
    let mut contents = Vec::new();
    let mut prefixes: Vec<String> = Vec::new();
    for key in sorted.into_iter().filter(|k| k.starts_with(prefix)) {
        let rolled = delimiter.and_then(|d| {
            key[prefix.len()..]
                .find(d)
                .map(|at| key[..prefix.len().saturating_add(at).saturating_add(d.len())].to_owned())
        });
        match rolled {
            Some(common) if prefixes.last() != Some(&common) => prefixes.push(common),
            Some(_) => {}
            None => contents.push(key.to_owned()),
        }
    }
    (contents, prefixes)
}

/// Pages a V2 listing to its end; returns (keys, common prefixes, pages).
async fn list_all(
    state: &tessari_s3_api::ApiState,
    bucket: &str,
    prefix: &str,
    delimiter: Option<&str>,
    max_keys: usize,
) -> (Vec<String>, Vec<String>, usize) {
    let max = max_keys.to_string();
    let (mut keys, mut prefixes) = (Vec::new(), Vec::new());
    let mut token: Option<String> = None;
    for page in 1..=200 {
        let mut query = vec![
            ("list-type", Some("2")),
            ("prefix", Some(prefix)),
            ("max-keys", Some(max.as_str())),
        ];
        if let Some(delimiter) = delimiter {
            query.push(("delimiter", Some(delimiter)));
        }
        if let Some(token) = &token {
            query.push(("continuation-token", Some(token.as_str())));
        }
        let seen = call(state, "GET", &format!("/{bucket}"), query, b"").await;
        assert_eq!(seen.status, 200, "{}", seen.body);
        let page_keys = texts(&seen.body, "Contents", "Key");
        let page_prefixes = texts(&seen.body, "CommonPrefixes", "Prefix");
        let count = page_keys.len().saturating_add(page_prefixes.len());
        assert!(count <= max_keys, "page {page} holds {count}");
        assert_eq!(
            text(&seen.body, "KeyCount"),
            Some(count.to_string()),
            "KeyCount counts keys and prefixes"
        );
        keys.extend(page_keys);
        prefixes.extend(page_prefixes);
        if text(&seen.body, "IsTruncated").as_deref() != Some("true") {
            return (keys, prefixes, page);
        }
        token = Some(
            text(&seen.body, "NextContinuationToken").expect("a truncated page carries a token"),
        );
    }
    panic!("the listing never ended");
}

const KEYS: [&str; 16] = [
    "a/b/c", "a/b/d", "a/x", "a b", "a", "b", "b0", "Z", "é", "€", "𝄞", "c&d<e>", "a/b/c/d", "a0",
    "d/", "d/e",
];

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn every_key_is_listed_once_in_byte_order_with_prefixes_rolled_up() {
    let _why = IGNORED;
    let (state, _) = fresh().await;
    bucket_with(&state, "list", &KEYS).await;
    bucket_with(&state, "lisu", &["a", "zz"]).await;
    for (prefix, delimiter) in [
        ("", None),
        ("", Some("/")),
        ("a", Some("/")),
        ("a/", Some("/")),
        ("a/b/", None),
    ] {
        let expected = reference(&KEYS, prefix, delimiter);
        for max_keys in [1000, 3, 1] {
            let (keys, prefixes, pages) =
                list_all(&state, "list", prefix, delimiter, max_keys).await;
            assert_eq!(
                (keys, prefixes),
                expected,
                "prefix {prefix:?} delimiter {delimiter:?} max-keys {max_keys}"
            );
            if max_keys == 1000 {
                assert_eq!(pages, 1);
            }
        }
    }
    assert!(
        reference(&KEYS, "", Some("/")).1.len() >= 2,
        "control: the reference rolls something up"
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_delete_between_pages_neither_skips_nor_repeats() {
    let (state, _) = fresh().await;
    let keys: Vec<String> = (0..10).map(|i| format!("k{i:02}")).collect();
    let refs: Vec<&str> = keys.iter().map(String::as_str).collect();
    bucket_with(&state, "pages", &refs).await;
    fn query(token: Option<&str>) -> Vec<(&'static str, Option<&str>)> {
        let mut query = vec![("list-type", Some("2")), ("max-keys", Some("3"))];
        if let Some(token) = token {
            query.push(("continuation-token", Some(token)));
        }
        query
    }
    let first = call(&state, "GET", "/pages", query(None), b"").await;
    assert_eq!(texts(&first.body, "Contents", "Key"), ["k00", "k01", "k02"]);
    let token = text(&first.body, "NextContinuationToken").expect("token");
    // The key the token resumes after, and one not yet returned, both go away.
    for gone in ["k02", "k05"] {
        assert_eq!(
            call(&state, "DELETE", &format!("/pages/{gone}"), vec![], b"")
                .await
                .status,
            204
        );
    }
    let mut seen: Vec<String> = texts(&first.body, "Contents", "Key");
    let mut next = Some(token);
    for _ in 0..10 {
        let Some(token) = next.take() else { break };
        let page = call(&state, "GET", "/pages", query(Some(&token)), b"").await;
        assert_eq!(page.status, 200, "{}", page.body);
        seen.extend(texts(&page.body, "Contents", "Key"));
        next = text(&page.body, "NextContinuationToken");
    }
    assert_eq!(
        seen,
        [
            "k00", "k01", "k02", "k03", "k04", "k06", "k07", "k08", "k09"
        ]
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn version_one_pages_by_marker_and_names_a_next_marker_only_with_a_delimiter() {
    let (state, _) = fresh().await;
    bucket_with(&state, "verone", &["a/1", "a/2", "b", "c", "d"]).await;
    let plain = call(&state, "GET", "/verone", vec![("max-keys", Some("2"))], b"").await;
    assert_eq!(plain.status, 200, "{}", plain.body);
    assert!(plain.body.contains("<ListBucketResult"), "{}", plain.body);
    assert_eq!(texts(&plain.body, "Contents", "Key"), ["a/1", "a/2"]);
    assert_eq!(text(&plain.body, "IsTruncated").as_deref(), Some("true"));
    assert_eq!(
        text(&plain.body, "NextMarker"),
        None,
        "no delimiter, no NextMarker"
    );
    assert!(plain.body.contains("<Owner>"), "V1 always names the owner");
    let resumed = call(
        &state,
        "GET",
        "/verone",
        vec![("max-keys", Some("2")), ("marker", Some("a/2"))],
        b"",
    )
    .await;
    assert_eq!(texts(&resumed.body, "Contents", "Key"), ["b", "c"]);
    let rolled = call(
        &state,
        "GET",
        "/verone",
        vec![("max-keys", Some("2")), ("delimiter", Some("/"))],
        b"",
    )
    .await;
    assert_eq!(texts(&rolled.body, "CommonPrefixes", "Prefix"), ["a/"]);
    assert_eq!(texts(&rolled.body, "Contents", "Key"), ["b"]);
    assert_eq!(text(&rolled.body, "NextMarker").as_deref(), Some("b"));
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn url_encoding_carries_any_key_and_without_it_an_unprintable_key_is_refused() {
    let (state, _) = fresh().await;
    bucket_with(&state, "enc", &["a b/ü+&", "plain"]).await;
    let encoded = call(
        &state,
        "GET",
        "/enc",
        vec![("list-type", Some("2")), ("encoding-type", Some("url"))],
        b"",
    )
    .await;
    assert_eq!(encoded.status, 200, "{}", encoded.body);
    assert_eq!(
        texts(&encoded.body, "Contents", "Key"),
        ["a+b/%C3%BC%2B%26", "plain"]
    );
    assert_eq!(text(&encoded.body, "EncodingType").as_deref(), Some("url"));
    let unprintable = "x\u{1}y";
    let put = call(
        &state,
        "PUT",
        &format!("/enc/{}", path_encode(unprintable)),
        vec![],
        b"1",
    )
    .await;
    assert_eq!(put.status, 200, "{}", put.body);
    let refused = call(&state, "GET", "/enc", vec![("list-type", Some("2"))], b"").await;
    assert_eq!(
        (refused.status, refused.code.as_deref()),
        (400, Some("InvalidArgument")),
        "{}",
        refused.body
    );
    assert!(refused.body.contains("encoding-type"), "{}", refused.body);
    let carried = call(
        &state,
        "GET",
        "/enc",
        vec![("list-type", Some("2")), ("encoding-type", Some("url"))],
        b"",
    )
    .await;
    assert!(
        texts(&carried.body, "Contents", "Key").contains(&"x%01y".to_owned()),
        "{}",
        carried.body
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_token_works_only_for_the_listing_it_was_issued_for() {
    let (state, _) = fresh().await;
    bucket_with(&state, "tok", &["a/1", "a/2", "a/3"]).await;
    bucket_with(&state, "tol", &["a/1", "a/2", "a/3"]).await;
    let first = call(
        &state,
        "GET",
        "/tok",
        vec![("list-type", Some("2")), ("max-keys", Some("1"))],
        b"",
    )
    .await;
    let token = text(&first.body, "NextContinuationToken").expect("token");
    let refused = |seen: &Seen| (seen.status, seen.code.clone());
    let elsewhere = [
        ("/tol", "", token.clone()),
        ("/tok", "a/", token.clone()),
        ("/tok", "", format!("{token}A")),
        ("/tok", "", "not-a-token".to_owned()),
    ];
    for (path, prefix, token) in elsewhere {
        let seen = call(
            &state,
            "GET",
            path,
            vec![
                ("list-type", Some("2")),
                ("prefix", Some(prefix)),
                ("continuation-token", Some(token.as_str())),
            ],
            b"",
        )
        .await;
        assert_eq!(
            refused(&seen),
            (400, Some("InvalidArgument".to_owned())),
            "{path} {prefix:?}: {}",
            seen.body
        );
    }
    let owner = call(
        &state,
        "GET",
        "/tok",
        vec![("list-type", Some("2")), ("fetch-owner", Some("true"))],
        b"",
    )
    .await;
    assert!(owner.body.contains("<Owner><ID>"), "{}", owner.body);
    let no_owner = call(&state, "GET", "/tok", vec![("list-type", Some("2"))], b"").await;
    assert!(!no_owner.body.contains("<Owner>"), "{}", no_owner.body);
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_missing_bucket_a_bad_max_keys_and_an_earlier_life_are_answered() {
    let (state, planter) = fresh().await;
    let missing = call(
        &state,
        "GET",
        "/nowhere",
        vec![("list-type", Some("2"))],
        b"",
    )
    .await;
    assert_eq!(
        (missing.status, missing.code.as_deref()),
        (404, Some("NoSuchBucket"))
    );
    bucket_with(&state, "life", &["old", "old2"]).await;
    for bad in ["-1", "ten", "1.5"] {
        let seen = call(
            &state,
            "GET",
            "/life",
            vec![("list-type", Some("2")), ("max-keys", Some(bad))],
            b"",
        )
        .await;
        assert_eq!(
            (seen.status, seen.code.as_deref()),
            (400, Some("InvalidArgument")),
            "max-keys {bad}"
        );
    }
    let capped = call(
        &state,
        "GET",
        "/life",
        vec![("list-type", Some("2")), ("max-keys", Some("5000"))],
        b"",
    )
    .await;
    assert_eq!(text(&capped.body, "MaxKeys").as_deref(), Some("1000"));
    // The bucket record goes and comes back: the object of its earlier life must not be listed.
    let name = vec![("name".to_owned(), Value::String("life".to_owned()))];
    planter
        .run("DELETE buckets:$name;", name)
        .await
        .expect("bucket record removed");
    assert_eq!(call(&state, "PUT", "/life", vec![], b"").await.status, 200);
    let reborn = call(&state, "GET", "/life", vec![("list-type", Some("2"))], b"").await;
    assert_eq!(reborn.status, 200, "{}", reborn.body);
    assert!(
        texts(&reborn.body, "Contents", "Key").is_empty(),
        "{}",
        reborn.body
    );
    assert_eq!(text(&reborn.body, "KeyCount").as_deref(), Some("0"));
    // A batch of max-keys + 1 = 2 records that are all of the earlier life must not end the listing.
    assert_eq!(
        call(&state, "PUT", "/life/zz", vec![], b"new").await.status,
        200
    );
    let past = call(
        &state,
        "GET",
        "/life",
        vec![("list-type", Some("2")), ("max-keys", Some("1"))],
        b"",
    )
    .await;
    assert_eq!(
        texts(&past.body, "Contents", "Key"),
        ["zz"],
        "{}",
        past.body
    );
}
