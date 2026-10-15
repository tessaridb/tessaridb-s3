//! The query string, decoded once per name and value. `?acl` has no value; `?acl=` has an empty one.

use tessari_s3_core::auth::percent_decode;
use tessari_s3_types::ErrorCode;

use crate::{Error, Result};

/// Decoded query parameters in arrival order.
///
/// # Errors
/// `InvalidArgument` for a broken escape or a name or value that is not UTF-8.
pub fn decode_query(raw_query: &str) -> Result<Vec<(String, Option<String>)>> {
    let invalid = || {
        Error::new(
            ErrorCode::InvalidArgument,
            "the query string is not valid percent-encoded UTF-8",
        )
    };
    let text = |raw: &str| {
        percent_decode(raw)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .ok_or_else(invalid)
    };
    raw_query
        .split('&')
        .filter(|parameter| !parameter.is_empty())
        .map(|parameter| match parameter.split_once('=') {
            Some((name, value)) => Ok((text(name)?, Some(text(value)?))),
            None => Ok((text(parameter)?, None)),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::decode_query;
    use tessari_s3_types::ErrorCode;

    #[test]
    fn names_and_values_decode_and_a_bare_name_has_no_value() {
        let decoded = decode_query("acl&prefix=a%2Fb&empty=&x-id=GetObject").expect("decodes");
        let expected = [
            ("acl", None),
            ("prefix", Some("a/b")),
            ("empty", Some("")),
            ("x-id", Some("GetObject")),
        ];
        let expected: Vec<(String, Option<String>)> = expected
            .iter()
            .map(|(n, v)| ((*n).to_owned(), v.map(str::to_owned)))
            .collect();
        assert_eq!(decoded, expected);
    }

    #[test]
    fn a_broken_escape_is_refused() {
        assert_eq!(
            decode_query("a=%zz").map_err(|e| e.code),
            Err(ErrorCode::InvalidArgument)
        );
    }
}
