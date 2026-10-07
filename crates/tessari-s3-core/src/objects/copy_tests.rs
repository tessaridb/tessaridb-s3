use super::{CopyRangeError, CopySource, CopySourceError, Directive, copy_range};

fn parsed(header: &str) -> Result<(String, String), CopySourceError> {
    CopySource::parse(header).map(|source| (source.bucket, source.key))
}

fn pair(bucket: &str, key: &str) -> Result<(String, String), CopySourceError> {
    Ok((bucket.to_owned(), key.to_owned()))
}

#[test]
fn a_copy_source_is_a_bucket_and_a_key_percent_decoded_once() {
    assert_eq!(parsed("photos/2026/a.jpg"), pair("photos", "2026/a.jpg"));
    assert_eq!(
        parsed("/photos/a.jpg"),
        pair("photos", "a.jpg"),
        "a leading slash"
    );
    assert_eq!(
        parsed("b/a%20b%3Fc%2Bd+e"),
        pair("b", "a b?c+d+e"),
        "decoded once; `+` is a plus"
    );
    assert_eq!(parsed("b/%C3%A9t%C3%A9"), pair("b", "été"));
    assert_eq!(parsed("b/a%2525"), pair("b", "a%25"), "never decoded twice");
    assert_eq!(
        parsed("b%2Fk"),
        pair("b", "k"),
        "the whole value is encoded, separator included"
    );
}

#[test]
fn a_version_is_not_implemented_and_anything_else_malformed() {
    assert_eq!(parsed("b/k?versionId=abc"), Err(CopySourceError::Version));
    for bad in [
        "", "/", "b", "b/", "/b/", "b/k?x=1", "b/%ZZ", "b/%FF", "b/k?",
    ] {
        assert_eq!(parsed(bad), Err(CopySourceError::Malformed), "{bad:?}");
    }
}

#[test]
fn a_directive_is_copy_unless_it_says_replace() {
    assert_eq!(Directive::parse(None), Some(Directive::Copy));
    assert_eq!(Directive::parse(Some("COPY")), Some(Directive::Copy));
    assert_eq!(Directive::parse(Some("REPLACE")), Some(Directive::Replace));
    for bad in ["copy", "replace", "", "MOVE"] {
        assert_eq!(Directive::parse(Some(bad)), None, "{bad:?}");
    }
}

#[test]
fn a_copy_range_is_first_to_last_inside_the_source() {
    assert_eq!(copy_range("bytes=0-9", 100), Ok((0, 9)));
    assert_eq!(copy_range("bytes=99-99", 100), Ok((99, 99)));
    assert_eq!(copy_range(" bytes=5-20 ", 100), Ok((5, 20)));
    for malformed in [
        "",
        "bytes=",
        "bytes=-5",
        "bytes=5-",
        "bytes=9-0",
        "bytes=0-1,2-3",
        "bytes=a-b",
        "0-9",
        "bytes=+1-2",
    ] {
        assert_eq!(
            copy_range(malformed, 100),
            Err(CopyRangeError::Malformed),
            "{malformed:?}"
        );
    }
    for past in [
        "bytes=0-100",
        "bytes=100-100",
        "bytes=0-18446744073709551615",
    ] {
        assert_eq!(
            copy_range(past, 100),
            Err(CopyRangeError::OutOfRange),
            "{past:?}"
        );
    }
    assert_eq!(
        copy_range("bytes=0-0", 0),
        Err(CopyRangeError::OutOfRange),
        "nothing to copy from"
    );
}
