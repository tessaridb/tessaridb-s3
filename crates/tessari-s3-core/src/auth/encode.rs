//! The byte encodings SigV4 is defined over: S3's `UriEncode`, percent-decoding of the raw request, lowercase hex.

use super::{AuthError, AuthResult};

/// Appends `input` to `out` encoded by the SigV4 `UriEncode` rules: every byte except `A-Z a-z 0-9 - . _ ~` becomes
/// `%XX` with UPPERCASE hex, a space is `%20` (never `+`), and `/` is kept only when `keep_slash` (the object key
/// path) and encoded everywhere else.
pub fn uri_encode(input: &[u8], keep_slash: bool, out: &mut String) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for &byte in input {
        let unreserved = byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~');
        if unreserved || (keep_slash && byte == b'/') {
            out.push(char::from(byte));
        } else {
            out.push('%');
            out.push(char::from(HEX[usize::from(byte >> 4)]));
            out.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }
}

/// Decodes `%XX` sequences in `raw` exactly once. `+` stays `+`: SigV4 clients encode a space as `%20`, so a literal
/// `+` on the wire is a plus sign.
///
/// # Errors
/// [`AuthError::InvalidPercentEncoding`] for a `%` not followed by two hex digits.
pub fn percent_decode(raw: &str) -> AuthResult<Vec<u8>> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut rest = bytes;
    while let Some((&byte, tail)) = rest.split_first() {
        if byte == b'%' {
            let (high, low) = match tail {
                [high, low, ..] => (hex_value(*high), hex_value(*low)),
                _ => (None, None),
            };
            let (Some(high), Some(low)) = (high, low) else {
                return Err(AuthError::InvalidPercentEncoding);
            };
            out.push((high << 4) | low);
            rest = tail.get(2..).unwrap_or_default();
        } else {
            out.push(byte);
            rest = tail;
        }
    }
    Ok(out)
}

/// Lowercase hex of `bytes`, the form every SigV4 hash and signature takes on the wire.
#[must_use]
pub fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len().saturating_mul(2));
    for &byte in bytes {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}

/// The value of one hex digit, either case.
const fn hex_value(digit: u8) -> Option<u8> {
    match digit {
        b'0'..=b'9' => Some(digit.wrapping_sub(b'0')),
        b'a'..=b'f' => Some(digit.wrapping_sub(b'a').wrapping_add(10)),
        b'A'..=b'F' => Some(digit.wrapping_sub(b'A').wrapping_add(10)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{hex_lower, percent_decode, uri_encode};
    use crate::auth::AuthError;

    fn encoded(input: &str, keep_slash: bool) -> String {
        let mut out = String::new();
        uri_encode(input.as_bytes(), keep_slash, &mut out);
        out
    }

    #[test]
    fn unreserved_bytes_pass_and_everything_else_is_uppercase_percent_hex() {
        assert_eq!(encoded("AZaz09-._~", false), "AZaz09-._~");
        assert_eq!(encoded("a b+c*d$e", false), "a%20b%2Bc%2Ad%24e");
        assert_eq!(encoded("é", false), "%C3%A9");
    }

    #[test]
    fn slash_is_kept_only_in_the_key_path() {
        assert_eq!(encoded("/a/b", true), "/a/b");
        assert_eq!(encoded("a/b", false), "a%2Fb");
    }

    #[test]
    fn decoding_happens_once_and_keeps_plus() {
        assert_eq!(percent_decode("a%2Fb%252F+c"), Ok(b"a/b%2F+c".to_vec()));
        assert_eq!(percent_decode("%c3%A9"), Ok("é".as_bytes().to_vec()));
    }

    #[test]
    fn a_broken_escape_is_refused() {
        for raw in ["%", "%4", "%zz", "a%g1"] {
            assert_eq!(
                percent_decode(raw),
                Err(AuthError::InvalidPercentEncoding),
                "{raw}"
            );
        }
    }

    #[test]
    fn hex_is_lowercase() {
        assert_eq!(hex_lower(&[0x00, 0xab, 0xff]), "00abff");
    }
}
