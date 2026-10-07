//! DeleteObjects against a real node: one result per key, Quiet, the integrity header S3 requires, and a request
//! whose shape this server does not implement refused before anything is deleted.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use tessari_s3_core::objects::checksum::{ChecksumAlgorithm, Hashes};

use crate::large::{body, data_of, queued};
use crate::{IGNORED, Seen, call, call_with, fresh};

const MIB: usize = 1 << 20;

fn md5_of(document: &str) -> String {
    let mut hashes = Hashes::new();
    hashes.update(document.as_bytes());
    STANDARD.encode(hashes.finish().md5)
}

fn document(keys: &[&str], quiet: bool) -> String {
    let objects: String = keys
        .iter()
        .map(|key| format!("<Object><Key>{key}</Key></Object>"))
        .collect();
    let quiet = if quiet { "<Quiet>true</Quiet>" } else { "" };
    format!("<Delete xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">{quiet}{objects}</Delete>")
}

async fn delete_many(state: &tessari_s3_api::ApiState, bucket: &str, document: &str) -> Seen {
    let md5 = md5_of(document);
    call_with(
        state,
        "POST",
        &format!("/{bucket}"),
        vec![("delete", None)],
        vec![("content-md5", md5.as_str())],
        document.as_bytes(),
    )
    .await
}

fn texts(body: &str, parent: &str, tag: &str) -> Vec<String> {
    let open = format!("<{parent}><{tag}>");
    let close = format!("</{tag}>");
    body.split(&open)
        .skip(1)
        .filter_map(|rest| {
            rest.split_once(&close)
                .map(|(text, _)| text.replace("&amp;", "&"))
        })
        .collect()
}

async fn status(state: &tessari_s3_api::ApiState, path: &str) -> u16 {
    call(state, "GET", path, vec![], b"").await.status
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn every_named_key_is_deleted_and_reported_and_the_rest_stay() {
    let _why = IGNORED;
    let (state, planter) = fresh().await;
    assert_eq!(call(&state, "PUT", "/many", vec![], b"").await.status, 200);
    for key in ["a", "b", "c%26d"] {
        assert_eq!(
            call(&state, "PUT", &format!("/many/{key}"), vec![], b"x")
                .await
                .status,
            200
        );
    }
    assert_eq!(
        call(&state, "PUT", "/many/big", vec![], &body(MIB + 1, 7))
            .await
            .status,
        200
    );
    let data = data_of(&planter, "many", "big")
        .await
        .expect("a data object");
    let seen = delete_many(
        &state,
        "many",
        &document(&["a", "c&amp;d", "missing", "big"], false),
    )
    .await;
    assert_eq!(seen.status, 200, "{}", seen.body);
    assert!(seen.body.contains("<DeleteResult"), "{}", seen.body);
    let mut deleted = texts(&seen.body, "Deleted", "Key");
    deleted.sort();
    assert_eq!(
        deleted,
        ["a", "big", "c&d", "missing"],
        "a missing key is deleted too, as in S3"
    );
    assert!(!seen.body.contains("<Error>"), "{}", seen.body);
    for gone in ["/many/a", "/many/c%26d", "/many/big"] {
        assert_eq!(status(&state, gone).await, 404, "{gone}");
    }
    assert_eq!(
        status(&state, "/many/b").await,
        200,
        "a key not named stays"
    );
    assert!(
        queued(&planter).await.contains(&data),
        "the deleted object's data is queued for reclamation"
    );
    let quiet = delete_many(&state, "many", &document(&["b"], true)).await;
    assert_eq!(quiet.status, 200, "{}", quiet.body);
    assert!(
        !quiet.body.contains("<Deleted>"),
        "Quiet reports errors only: {}",
        quiet.body
    );
    assert_eq!(status(&state, "/many/b").await, 404);
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn the_integrity_header_s3_requires_is_required_and_checked() {
    let (state, _) = fresh().await;
    assert_eq!(call(&state, "PUT", "/proof", vec![], b"").await.status, 200);
    assert_eq!(
        call(&state, "PUT", "/proof/k", vec![], b"x").await.status,
        200
    );
    let doc = document(&["k"], false);
    let refused = |seen: &Seen| (seen.status, seen.code.clone());
    let bare = call(
        &state,
        "POST",
        "/proof",
        vec![("delete", None)],
        doc.as_bytes(),
    )
    .await;
    assert_eq!(
        refused(&bare),
        (400, Some("InvalidRequest".to_owned())),
        "{}",
        bare.body
    );
    let wrong = call_with(
        &state,
        "POST",
        "/proof",
        vec![("delete", None)],
        vec![("content-md5", md5_of("another document").as_str())],
        doc.as_bytes(),
    )
    .await;
    assert_eq!(
        refused(&wrong),
        (400, Some("BadDigest".to_owned())),
        "{}",
        wrong.body
    );
    assert_eq!(
        status(&state, "/proof/k").await,
        200,
        "nothing deleted by a refused request"
    );
    // What current SDKs send instead of Content-MD5: a CRC32 checksum header.
    let mut hashes = Hashes::new();
    hashes.update(doc.as_bytes());
    let crc32 = hashes.finish().value(ChecksumAlgorithm::Crc32).to_owned();
    let with_crc = call_with(
        &state,
        "POST",
        "/proof",
        vec![("delete", None)],
        vec![("x-amz-checksum-crc32", crc32.as_str())],
        doc.as_bytes(),
    )
    .await;
    assert_eq!(with_crc.status, 200, "{}", with_crc.body);
    assert_eq!(status(&state, "/proof/k").await, 404);
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_request_this_server_cannot_honour_deletes_nothing() {
    let (state, _) = fresh().await;
    assert_eq!(call(&state, "PUT", "/keep", vec![], b"").await.status, 200);
    assert_eq!(
        call(&state, "PUT", "/keep/k", vec![], b"x").await.status,
        200
    );
    let too_many: Vec<String> = (0..1001).map(|i| format!("k{i}")).collect();
    let refs: Vec<&str> = too_many.iter().map(String::as_str).collect();
    let cases = [
        (document(&refs, false), 400, "MalformedXML"),
        (
            "<Delete><Object><Key>k</Key><VersionId>v1</VersionId></Object></Delete>".to_owned(),
            501,
            "NotImplemented",
        ),
        (
            "<Delete><Object><Key>k</Key><ETag>\"x\"</ETag></Object></Delete>".to_owned(),
            501,
            "NotImplemented",
        ),
        ("<Delete></Delete>".to_owned(), 400, "MalformedXML"),
        (
            "<Delete><Object><Key>k</Key></Object>".to_owned(),
            400,
            "MalformedXML",
        ),
        (
            "<!DOCTYPE d [<!ENTITY e \"k\">]><Delete><Object><Key>&e;</Key></Object></Delete>"
                .to_owned(),
            400,
            "MalformedXML",
        ),
    ];
    for (doc, code_status, code) in cases {
        let seen = delete_many(&state, "keep", &doc).await;
        assert_eq!(
            (seen.status, seen.code.as_deref()),
            (code_status, Some(code)),
            "{}: {}",
            &doc[..doc.len().min(80)],
            seen.body
        );
        assert_eq!(status(&state, "/keep/k").await, 200, "nothing deleted");
    }
    let nowhere = delete_many(&state, "nowhere", &document(&["k"], false)).await;
    assert_eq!(
        (nowhere.status, nowhere.code.as_deref()),
        (404, Some("NoSuchBucket"))
    );
}
