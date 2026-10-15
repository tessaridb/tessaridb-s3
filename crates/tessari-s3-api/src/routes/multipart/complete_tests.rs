use tessari_s3_core::objects::checksum::ChecksumAlgorithm;
use tessari_s3_types::ErrorCode;

use super::parse;

fn code(document: &str) -> Option<ErrorCode> {
    parse(document.as_bytes()).err().map(|error| error.code)
}

#[test]
fn parts_come_back_as_listed_with_their_etags_and_checksums() {
    let document = "<?xml version=\"1.0\"?>\n<CompleteMultipartUpload xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\n\
                    <Part><PartNumber>2</PartNumber><ETag>&quot;b&quot;</ETag>\
                    <ChecksumCRC32>AAAAAA==</ChecksumCRC32></Part>\n\
                    <Part><ETag>\"a\"</ETag><PartNumber> 1 </PartNumber></Part></CompleteMultipartUpload>";
    let parts = parse(document.as_bytes()).expect("parses");
    assert_eq!(
        parts
            .iter()
            .map(|p| (p.number, p.etag.as_str()))
            .collect::<Vec<_>>(),
        [(2, "\"b\""), (1, "\"a\"")],
        "kept in the listed order, entities resolved"
    );
    assert_eq!(
        parts[0].checksums,
        [(ChecksumAlgorithm::Crc32, "AAAAAA==".to_owned())]
    );
}

#[test]
fn ten_thousand_parts_are_read_and_one_more_is_malformed() {
    let document = |count: usize| {
        let parts: String = (0..count)
            .map(|_| "<Part><PartNumber>1</PartNumber><ETag>e</ETag></Part>")
            .collect();
        format!("<CompleteMultipartUpload>{parts}</CompleteMultipartUpload>")
    };
    assert_eq!(
        parse(document(10_000).as_bytes()).map(|p| p.len()),
        Ok(10_000)
    );
    assert_eq!(code(&document(10_001)), Some(ErrorCode::MalformedXml));
}

#[test]
fn an_unimplemented_checksum_is_refused_by_name_and_anything_else_is_malformed() {
    assert_eq!(
        code(
            "<CompleteMultipartUpload><Part><PartNumber>1</PartNumber><ETag>e</ETag>\
              <ChecksumSHA512>x</ChecksumSHA512></Part></CompleteMultipartUpload>"
        ),
        Some(ErrorCode::NotImplemented)
    );
    for document in [
        "",
        "<CompleteMultipartUpload></CompleteMultipartUpload>",
        "<CompleteMultipartUpload><Part><ETag>e</ETag></Part></CompleteMultipartUpload>",
        "<CompleteMultipartUpload><Part><PartNumber>1</PartNumber></Part></CompleteMultipartUpload>",
        "<CompleteMultipartUpload><Part><PartNumber>0</PartNumber><ETag>e</ETag></Part></CompleteMultipartUpload>",
        "<CompleteMultipartUpload><Part><PartNumber>10001</PartNumber><ETag>e</ETag></Part></CompleteMultipartUpload>",
        "<CompleteMultipartUpload><Part><PartNumber>1</PartNumber><ETag>e</ETag><Size>1</Size></Part>\
         </CompleteMultipartUpload>",
        "<Complete><Part><PartNumber>1</PartNumber><ETag>e</ETag></Part></Complete>",
        "<CompleteMultipartUpload><Part><PartNumber>1</PartNumber><ETag>&e;</ETag></Part></CompleteMultipartUpload>",
        "<!DOCTYPE d [<!ENTITY e \"x\">]><CompleteMultipartUpload><Part><PartNumber>1</PartNumber>\
         <ETag>&e;</ETag></Part></CompleteMultipartUpload>",
        "<CompleteMultipartUpload><Part><PartNumber>1</PartNumber><ETag>e</ETag></Part>",
    ] {
        assert_eq!(code(document), Some(ErrorCode::MalformedXml), "{document}");
    }
}
