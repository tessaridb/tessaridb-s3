//! XML in and out. Output is built from escaped text; input is read with a pull parser that never resolves a
//! document type or an entity beyond the five predefined ones, and refuses a body that declares a DTD at all.

use quick_xml::Reader;
use quick_xml::events::Event;
use tessari_s3_types::ErrorCode;

use crate::Error;

/// The namespace every S3 response document declares.
pub const S3_NAMESPACE: &str = "http://s3.amazonaws.com/doc/2006-03-01/";

/// Escapes the five XML special characters.
#[must_use]
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

/// The text of the first `<element>` under a root called `root`; `Ok(None)` for an empty body or an absent element.
/// Element names compare by local name, so a body with or without the S3 namespace reads the same.
///
/// # Errors
/// `MalformedXML` for a body that is not well-formed, has another root, or declares a DTD.
pub fn element_text(body: &[u8], root: &str, element: &str) -> Result<Option<String>, Error> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(None);
    }
    let malformed = || {
        Error::new(
            ErrorCode::MalformedXml,
            "the XML body is not well formed or not the expected shape",
        )
    };
    let mut reader = Reader::from_reader(body);
    let mut depth: usize = 0;
    let mut inside = false;
    let mut found = None;
    loop {
        match reader.read_event().map_err(|_| malformed())? {
            Event::DocType(_) => return Err(malformed()),
            Event::Start(start) => {
                let name = start.local_name();
                if depth == 0 && name.as_ref() != root {
                    return Err(malformed());
                }
                inside = found.is_none() && name.as_ref() == element;
                depth = depth.checked_add(1).ok_or_else(malformed)?;
            }
            Event::Empty(empty) => {
                if depth == 0 && empty.local_name().as_ref() != root {
                    return Err(malformed());
                }
                if found.is_none() && empty.local_name().as_ref() == element {
                    found = Some(String::new());
                }
            }
            Event::Text(text) if inside => found = Some(text.xml10_content().trim().to_owned()),
            // No value this parser reads contains an entity, so a reference inside the element is refused rather
            // than expanded.
            Event::GeneralRef(_) if inside => return Err(malformed()),
            Event::End(_) => {
                inside = false;
                depth = depth.checked_sub(1).ok_or_else(malformed)?;
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if depth == 0 {
        Ok(found)
    } else {
        Err(malformed())
    }
}

#[cfg(test)]
mod tests {
    use super::{element_text, escape};
    use tessari_s3_types::ErrorCode;

    const ROOT: &str = "CreateBucketConfiguration";

    #[test]
    fn the_location_constraint_is_read_with_or_without_the_namespace() {
        let with_ns = br#"<CreateBucketConfiguration xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><LocationConstraint>eu-west-1</LocationConstraint></CreateBucketConfiguration>"#;
        let without = b"<CreateBucketConfiguration><LocationConstraint> eu-west-1 </LocationConstraint></CreateBucketConfiguration>";
        for body in [&with_ns[..], &without[..]] {
            assert_eq!(
                element_text(body, ROOT, "LocationConstraint"),
                Ok(Some("eu-west-1".to_owned()))
            );
        }
        assert_eq!(element_text(b"", ROOT, "LocationConstraint"), Ok(None));
        assert_eq!(
            element_text(b"<CreateBucketConfiguration/>", ROOT, "LocationConstraint"),
            Ok(None)
        );
    }

    #[test]
    fn a_dtd_another_root_or_broken_markup_is_malformed() {
        let cases: [&[u8]; 4] = [
            b"<!DOCTYPE x [<!ENTITY e SYSTEM \"file:///etc/passwd\">]><CreateBucketConfiguration>&e;</CreateBucketConfiguration>",
            b"<Other><LocationConstraint>x</LocationConstraint></Other>",
            b"<CreateBucketConfiguration><LocationConstraint>x</CreateBucketConfiguration>",
            b"<CreateBucketConfiguration>",
        ];
        for body in cases {
            assert_eq!(
                element_text(body, ROOT, "LocationConstraint").map_err(|e| e.code),
                Err(ErrorCode::MalformedXml),
                "{}",
                String::from_utf8_lossy(body)
            );
        }
    }

    #[test]
    fn markup_cannot_escape_an_element() {
        assert_eq!(escape("a<b>&\"c'"), "a&lt;b&gt;&amp;&quot;c&apos;");
    }
}
