use tessari_s3_types::ErrorCode;

use super::{Request, parse};

fn keys(request: &Request) -> Vec<&str> {
    request
        .entries
        .iter()
        .map(|(key, _)| key.as_str())
        .collect()
}

#[test]
fn a_version_id_comes_back_beside_its_key() {
    let request = parse(b"<Delete><Object><Key>k</Key><VersionId>v1</VersionId></Object><Object><Key>j</Key></Object></Delete>")
        .expect("parses");
    let versions: Vec<Option<&str>> = request
        .entries
        .iter()
        .map(|(_, version)| version.as_deref())
        .collect();
    assert_eq!(
        (keys(&request), versions),
        (vec!["k", "j"], vec![Some("v1"), None])
    );
    assert_eq!(
        code(
            "<Delete><Object><Key>k</Key><VersionId>a</VersionId><VersionId>b</VersionId></Object></Delete>"
        ),
        Some(ErrorCode::MalformedXml),
        "one version per entry"
    );
}

fn code(document: &str) -> Option<ErrorCode> {
    parse(document.as_bytes()).err().map(|error| error.code)
}

#[test]
fn keys_come_back_with_their_entities_and_character_references_resolved() {
    let document = "<?xml version=\"1.0\"?>\n<Delete xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\n\
                    <Quiet>true</Quiet>\n<Object><Key>c&amp;d&lt;e&gt;</Key></Object>\n\
                    <Object><Key>a&#13;b&#x20AC;</Key></Object><Object><Key><![CDATA[x<y]]></Key></Object></Delete>";
    let request = parse(document.as_bytes()).expect("parses");
    assert!(request.quiet);
    assert_eq!(keys(&request), ["c&d<e>", "a\rb€", "x<y"]);
    let plain = parse(b"<Delete><Object><Key>k</Key></Object></Delete>").expect("parses");
    assert!(!plain.quiet, "Quiet defaults to false");
}

#[test]
fn a_thousand_keys_are_accepted_and_a_thousand_and_one_are_malformed() {
    let document = |count: usize| {
        let objects: String = (0..count)
            .map(|i| format!("<Object><Key>k{i}</Key></Object>"))
            .collect();
        format!("<Delete>{objects}</Delete>")
    };
    assert_eq!(
        parse(document(1000).as_bytes()).map(|r| r.entries.len()),
        Ok(1000)
    );
    assert_eq!(code(&document(1001)), Some(ErrorCode::MalformedXml));
}

#[test]
fn what_is_not_implemented_is_refused_by_name() {
    for element in [
        "<ETag>\"e\"</ETag>",
        "<LastModifiedTime>2026-01-01T00:00:00Z</LastModifiedTime>",
        "<Size>1</Size>",
    ] {
        let document = format!("<Delete><Object><Key>k</Key>{element}</Object></Delete>");
        assert_eq!(
            code(&document),
            Some(ErrorCode::NotImplemented),
            "{element}"
        );
    }
}

#[test]
fn anything_else_is_malformed() {
    for document in [
        "",
        "<Delete></Delete>",
        "<Delete><Object></Object></Delete>",
        "<Delete><Object><Key></Key></Object></Delete>",
        "<Delete><Object><Key>k</Key></Object>",
        "<Delete><Object><Key>k</Key></Object></Delete><Delete/>",
        "<Other><Object><Key>k</Key></Object></Other>",
        "<Delete><Object><Key>k</Key><Extra>x</Extra></Object></Delete>",
        "<Delete><Quiet>yes</Quiet><Object><Key>k</Key></Object></Delete>",
        "<Delete>text<Object><Key>k</Key></Object></Delete>",
        "<Delete><Object><Key>k&e;</Key></Object></Delete>",
        "<Delete><Object><Key>k&#1;</Key></Object></Delete>",
        "<!DOCTYPE d [<!ENTITY e \"k\">]><Delete><Object><Key>&e;</Key></Object></Delete>",
    ] {
        assert_eq!(code(document), Some(ErrorCode::MalformedXml), "{document}");
    }
}
