//! CompleteMultipartUpload and reads of the object it makes: only the listed parts, checked before anything is
//! visible, become the object in one commit; it reads back across its part files; its parts' files are queued the
//! moment nothing references them any more.

use tessari_s3_core::objects::checksum::{ChecksumAlgorithm, Hashes};

use crate::large::{body, queued};
use crate::multipart::{create, etag, md5_hex, part, part_data, text, texts};
use crate::{IGNORED, Seen, call, call_with, fresh};

const MIB: usize = 1 << 20;

fn header<'a>(seen: &'a Seen, name: &str) -> Option<&'a str> {
    seen.headers.get(name).and_then(|v| v.to_str().ok())
}

/// The Complete document for `(number, etag)` pairs, in the given order.
fn document(parts: &[(&str, &str)]) -> Vec<u8> {
    let listed: String = parts
        .iter()
        .map(|(number, etag)| {
            format!("<Part><PartNumber>{number}</PartNumber><ETag>{etag}</ETag></Part>")
        })
        .collect();
    format!("<CompleteMultipartUpload xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">{listed}</CompleteMultipartUpload>")
        .into_bytes()
}

async fn complete(
    state: &tessari_s3_api::ApiState,
    path: &str,
    upload: &str,
    headers: Vec<(&str, &str)>,
    document: &[u8],
) -> Seen {
    call_with(
        state,
        "POST",
        path,
        vec![("uploadId", Some(upload))],
        headers,
        document,
    )
    .await
}

/// The multipart ETag S3 publishes: MD5 over the parts' binary MD5s, then `-` and the count.
fn multipart_etag(parts: &[&[u8]]) -> String {
    let mut concatenated = Vec::new();
    for bytes in parts {
        let mut hashes = Hashes::new();
        hashes.update(bytes);
        concatenated.extend_from_slice(&hashes.finish().md5);
    }
    format!("\"{}-{}\"", md5_hex(&concatenated), parts.len())
}

fn checksum(bytes: &[u8], algorithm: ChecksumAlgorithm) -> String {
    let mut hashes = Hashes::new();
    hashes.update(bytes);
    hashes.finish().value(algorithm).to_owned()
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn complete_makes_the_listed_parts_one_object_that_reads_back_across_them() {
    let _why = IGNORED;
    let (state, planter) = fresh().await;
    assert_eq!(
        call(&state, "PUT", "/mpdone", vec![], b"").await.status,
        200
    );
    let upload = create(&state, "/mpdone/big").await;
    let (first, second, last) = (
        body(5 * MIB, 1),
        body(5 * MIB, 2),
        b"the tail of it".to_vec(),
    );
    let mut etags = Vec::new();
    for (number, bytes) in [
        ("1", &first),
        ("2", &second),
        ("3", &last),
        ("4", &b"never listed".to_vec()),
    ] {
        let seen = part(&state, "/mpdone/big", &upload, number, bytes).await;
        assert_eq!(seen.status, 200, "{}", seen.body);
        etags.push(etag(&seen));
    }
    let before = part_data(&planter).await;
    assert_eq!(before.len(), 4);
    let done = complete(
        &state,
        "/mpdone/big",
        &upload,
        Vec::new(),
        &document(&[("1", &etags[0]), ("2", &etags[1]), ("3", &etags[2])]),
    )
    .await;
    assert_eq!(done.status, 200, "{}", done.body);
    let expected_etag = multipart_etag(&[&first, &second, &last]);
    assert_eq!(
        text(&done.body, "ETag"),
        Some(expected_etag.clone()),
        "{}",
        done.body
    );
    assert_eq!(text(&done.body, "Key").as_deref(), Some("big"));
    // The unlisted part left with the upload, and its file was queued.
    let after = part_data(&planter).await;
    assert_eq!(
        after.len(),
        3,
        "only the listed parts remain, as the object's"
    );
    let dropped: Vec<[u8; 16]> = before
        .iter()
        .filter(|id| !after.contains(id))
        .copied()
        .collect();
    assert_eq!(dropped.len(), 1);
    assert!(
        queued(&planter).await.contains(&dropped[0]),
        "the unlisted part's file was queued"
    );
    let gone = call(
        &state,
        "GET",
        "/mpdone/big",
        vec![("uploadId", Some(upload.as_str()))],
        b"",
    )
    .await;
    assert_eq!(
        gone.code.as_deref(),
        Some("NoSuchUpload"),
        "the upload ended with the commit"
    );
    // The whole object, byte for byte, with its multipart ETag and a full-object CRC64NVME.
    let whole: Vec<u8> = [first.as_slice(), second.as_slice(), last.as_slice()].concat();
    let read = call_with(
        &state,
        "GET",
        "/mpdone/big",
        vec![],
        vec![("x-amz-checksum-mode", "ENABLED")],
        b"",
    )
    .await;
    assert_eq!(read.status, 200, "{}", read.body);
    assert!(
        read.bytes == whole,
        "the object is the listed parts in order"
    );
    assert_eq!(header(&read, "etag"), Some(expected_etag.as_str()));
    assert_eq!(
        header(&read, "x-amz-checksum-crc64nvme"),
        Some(checksum(&whole, ChecksumAlgorithm::Crc64Nvme).as_str()),
        "the full-object CRC64NVME combined from the parts' CRCs"
    );
    assert_eq!(header(&read, "x-amz-checksum-type"), Some("FULL_OBJECT"));
    let head = call(&state, "HEAD", "/mpdone/big", vec![], b"").await;
    assert_eq!(
        header(&head, "content-length"),
        Some(whole.len().to_string().as_str())
    );
    // A range straddling the first part boundary.
    let (from, to) = (5 * MIB - 10, 5 * MIB + 9);
    let range = format!("bytes={from}-{to}");
    let ranged = call_with(
        &state,
        "GET",
        "/mpdone/big",
        vec![],
        vec![("range", range.as_str())],
        b"",
    )
    .await;
    assert_eq!(ranged.status, 206, "{}", ranged.body);
    assert!(
        ranged.bytes == whole[from..=to],
        "the range crosses into the second part's file"
    );
    // One part by number.
    let by_part = call(
        &state,
        "GET",
        "/mpdone/big",
        vec![("partNumber", Some("2"))],
        b"",
    )
    .await;
    assert_eq!(by_part.status, 206, "{}", by_part.body);
    assert!(by_part.bytes == second, "partNumber=2 is the second part");
    assert_eq!(header(&by_part, "x-amz-mp-parts-count"), Some("3"));
    let listing = call(
        &state,
        "GET",
        "/mpdone",
        vec![("list-type", Some("2"))],
        b"",
    )
    .await;
    assert_eq!(
        texts(&listing.body, "Contents", "Size"),
        [whole.len().to_string()]
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn complete_refuses_a_list_it_cannot_honour_and_leaves_the_upload_intact() {
    let (state, _) = fresh().await;
    assert_eq!(call(&state, "PUT", "/mpbad", vec![], b"").await.status, 200);
    let upload = create(&state, "/mpbad/k").await;
    let mut etags = Vec::new();
    for (number, bytes) in [
        ("1", body(5 * MIB, 3)),
        ("2", b"small".to_vec()),
        ("3", b"end".to_vec()),
    ] {
        etags.push(etag(
            &part(&state, "/mpbad/k", &upload, number, &bytes).await,
        ));
    }
    let wrong = format!("\"{}\"", md5_hex(b"not the part"));
    let cases: [(&str, Vec<u8>, &str); 5] = [
        (
            "descending",
            document(&[("2", &etags[1]), ("1", &etags[0])]),
            "InvalidPartOrder",
        ),
        (
            "wrong ETag",
            document(&[("1", &wrong), ("3", &etags[2])]),
            "InvalidPart",
        ),
        (
            "missing part",
            document(&[("1", &etags[0]), ("9", &etags[2])]),
            "InvalidPart",
        ),
        (
            "small middle part",
            document(&[("2", &etags[1]), ("3", &etags[2])]),
            "EntityTooSmall",
        ),
        (
            "not the document",
            b"<Complete><Part/></Complete>".to_vec(),
            "MalformedXML",
        ),
    ];
    for (case, document, code) in cases {
        let seen = complete(&state, "/mpbad/k", &upload, Vec::new(), &document).await;
        assert_eq!(
            (seen.status, seen.code.as_deref()),
            (400, Some(code)),
            "{case}: {}",
            seen.body
        );
    }
    let still = call(
        &state,
        "GET",
        "/mpbad/k",
        vec![("uploadId", Some(upload.as_str()))],
        b"",
    )
    .await;
    assert_eq!(
        texts(&still.body, "Part", "PartNumber"),
        ["1", "2", "3"],
        "nothing was taken"
    );
    assert_eq!(
        call(&state, "GET", "/mpbad/k", vec![], b"")
            .await
            .code
            .as_deref(),
        Some("NoSuchKey")
    );
    let forged = complete(
        &state,
        "/mpbad/k",
        &"0".repeat(32),
        Vec::new(),
        &document(&[("1", &etags[0])]),
    )
    .await;
    assert_eq!(
        (forged.status, forged.code.as_deref()),
        (404, Some("NoSuchUpload"))
    );
    let other_key = complete(
        &state,
        "/mpbad/other",
        &upload,
        Vec::new(),
        &document(&[("1", &etags[0])]),
    )
    .await;
    assert_eq!(
        other_key.code.as_deref(),
        Some("NoSuchUpload"),
        "the upload is bound to its key"
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn complete_honours_its_condition_and_an_overwrite_releases_every_part() {
    let (state, planter) = fresh().await;
    assert_eq!(
        call(&state, "PUT", "/mpcond", vec![], b"").await.status,
        200
    );
    assert_eq!(
        call(&state, "PUT", "/mpcond/k", vec![], b"already here")
            .await
            .status,
        200
    );
    let upload = create(&state, "/mpcond/k").await;
    let first = etag(&part(&state, "/mpcond/k", &upload, "1", &body(5 * MIB, 4)).await);
    let second = etag(&part(&state, "/mpcond/k", &upload, "2", b"two").await);
    let listed = document(&[("1", &first), ("2", &second)]);
    let refused = complete(
        &state,
        "/mpcond/k",
        &upload,
        vec![("if-none-match", "*")],
        &listed,
    )
    .await;
    assert_eq!(
        (refused.status, refused.code.as_deref()),
        (412, Some("PreconditionFailed")),
        "{}",
        refused.body
    );
    assert_eq!(
        call(&state, "GET", "/mpcond/k", vec![], b"").await.bytes,
        b"already here"
    );
    let done = complete(&state, "/mpcond/k", &upload, Vec::new(), &listed).await;
    assert_eq!(done.status, 200, "{}", done.body);
    let parts = part_data(&planter).await;
    assert_eq!(parts.len(), 2);
    assert_eq!(
        call(&state, "PUT", "/mpcond/k", vec![], b"replaced")
            .await
            .status,
        200
    );
    assert!(
        part_data(&planter).await.is_empty(),
        "the overwrite dropped the object's parts"
    );
    let queued_now = queued(&planter).await;
    assert!(
        parts.iter().all(|id| queued_now.contains(id)),
        "and queued every part's file"
    );
}

#[tokio::test]
#[ignore = "needs a TessariDB node: set TESSARIDB_S3_TEST_META, _USER and _PASSWORD"]
async fn a_declared_composite_checksum_is_the_checksum_of_the_part_checksums() {
    let (state, _) = fresh().await;
    assert_eq!(call(&state, "PUT", "/mpsum", vec![], b"").await.status, 200);
    let created = call_with(
        &state,
        "POST",
        "/mpsum/k",
        vec![("uploads", None)],
        vec![("x-amz-checksum-algorithm", "SHA256")],
        b"",
    )
    .await;
    assert_eq!(created.status, 200, "{}", created.body);
    let upload = text(&created.body, "UploadId").expect("an upload id");
    let (first, second) = (body(5 * MIB, 5), b"second".to_vec());
    let one = etag(&part(&state, "/mpsum/k", &upload, "1", &first).await);
    let two = etag(&part(&state, "/mpsum/k", &upload, "2", &second).await);
    let done = complete(
        &state,
        "/mpsum/k",
        &upload,
        Vec::new(),
        &document(&[("1", &one), ("2", &two)]),
    )
    .await;
    assert_eq!(done.status, 200, "{}", done.body);
    let mut raw = Vec::new();
    for bytes in [&first, &second] {
        let mut hashes = Hashes::new();
        hashes.update(bytes);
        raw.extend_from_slice(&hashes.finish().sha256);
    }
    let expected = format!("{}-2", checksum(&raw, ChecksumAlgorithm::Sha256));
    let read = call_with(
        &state,
        "HEAD",
        "/mpsum/k",
        vec![],
        vec![("x-amz-checksum-mode", "ENABLED")],
        b"",
    )
    .await;
    assert_eq!(
        header(&read, "x-amz-checksum-sha256"),
        Some(expected.as_str())
    );
    assert_eq!(header(&read, "x-amz-checksum-type"), Some("COMPOSITE"));
}
