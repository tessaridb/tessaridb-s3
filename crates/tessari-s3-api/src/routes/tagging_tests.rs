use tessari_s3_core::tagging::{BUCKET_TAGS, OBJECT_TAGS};
use tessari_s3_types::ErrorCode;

use super::{document, parse};

#[test]
fn a_tagging_document_reads_back_as_written() {
    let tags = parse(
        b"<Tagging xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><TagSet>\
          <Tag><Key>team</Key><Value>blue green</Value></Tag><Tag><Key>tier</Key></Tag>\
          </TagSet></Tagging>",
        OBJECT_TAGS,
    )
    .expect("parses");
    assert_eq!(
        tags.iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect::<Vec<_>>(),
        [("team", "blue green"), ("tier", "")]
    );
    assert_eq!(
        parse(document(&tags).as_bytes(), OBJECT_TAGS).expect("round trip"),
        tags,
        "a written document parses back to the same tags"
    );
}

#[test]
fn another_shape_is_malformed_and_a_refused_set_is_an_invalid_tag() {
    for body in [
        &b"<Tags><TagSet></TagSet></Tags>"[..],
        b"<Tagging><Other/></Tagging>",
        b"<Tagging><TagSet><Item/></TagSet></Tagging>",
        b"<Tagging><TagSet><Tag><Value>v</Value></Tag></TagSet></Tagging>",
        b"<Tagging><TagSet><Tag><Key>a</Key><Key>b</Key></Tag></TagSet></Tagging>",
        b"<Tagging><TagSet><Tag><Key>a</Key><Owner>x</Owner></Tag></TagSet></Tagging>",
        b"<!DOCTYPE x [<!ENTITY e \"x\">]><Tagging><TagSet/></Tagging>",
    ] {
        assert_eq!(
            parse(body, OBJECT_TAGS)
                .map(|_| ())
                .expect_err("refused")
                .code,
            ErrorCode::MalformedXml,
            "{}",
            String::from_utf8_lossy(body)
        );
    }
    let many: String = (0..11)
        .map(|n| format!("<Tag><Key>k{n}</Key><Value></Value></Tag>"))
        .collect();
    let body = format!("<Tagging><TagSet>{many}</TagSet></Tagging>");
    assert_eq!(
        parse(body.as_bytes(), OBJECT_TAGS)
            .map(|_| ())
            .expect_err("refused")
            .code,
        ErrorCode::InvalidTag
    );
    assert!(
        parse(body.as_bytes(), BUCKET_TAGS).is_ok(),
        "a bucket holds eleven"
    );
}
