//! Which bucket and key a request addresses: virtual-hosted (`<bucket>.<domain>`) when the Host ends in a
//! configured endpoint domain, path-style (`/<bucket>/<key>`) otherwise. The key is the raw path decoded ONCE and
//! never normalised: `a//b`, `a/./b` and `a/b` are three keys.

use tessari_s3_core::auth::percent_decode;
use tessari_s3_core::dispatch::{CATALOG, Target};
use tessari_s3_types::{BucketName, ErrorCode, ObjectKey};

use crate::{Error, Result};

/// What a request path addresses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Addressed {
    /// The dispatch target.
    pub target: Target,
    /// The bucket, for bucket and object requests.
    pub bucket: Option<BucketName>,
    /// The key, for object requests.
    pub key: Option<ObjectKey>,
}

/// Resolves the bucket and key from the Host header and the raw (still percent-encoded) path.
///
/// # Errors
/// `InvalidBucketName`, `KeyTooLongError` or `InvalidURI`.
pub fn resolve(host: &str, raw_path: &str, domains: &[String]) -> Result<Addressed> {
    let host = strip_port(host).to_ascii_lowercase();
    let virtual_bucket = domains.iter().find_map(|domain| {
        host.strip_suffix(domain.as_str())
            .and_then(|prefix| prefix.strip_suffix('.'))
            .filter(|b| !b.is_empty())
    });
    let (bucket_text, raw_key) = if let Some(bucket) = virtual_bucket {
        (
            bucket.to_owned(),
            raw_path.strip_prefix('/').unwrap_or(raw_path),
        )
    } else {
        if let Some(named) = named_target(raw_path) {
            return Ok(Addressed {
                target: named,
                bucket: None,
                key: None,
            });
        }
        let trimmed = raw_path.strip_prefix('/').unwrap_or(raw_path);
        let (raw_bucket, raw_key) = trimmed.split_once('/').unwrap_or((trimmed, ""));
        (decode(raw_bucket)?, raw_key)
    };
    if bucket_text.is_empty() {
        return Ok(Addressed {
            target: Target::Service,
            bucket: None,
            key: None,
        });
    }
    let bucket = BucketName::new(&bucket_text)
        .map_err(|e| Error::new(ErrorCode::InvalidBucketName, e.to_string()))?;
    if raw_key.is_empty() {
        return Ok(Addressed {
            target: Target::Bucket,
            bucket: Some(bucket),
            key: None,
        });
    }
    let key = ObjectKey::new(&decode(raw_key)?)
        .map_err(|e| Error::new(ErrorCode::KeyTooLongError, e.to_string()))?;
    Ok(Addressed {
        target: Target::Object,
        bucket: Some(bucket),
        key: Some(key),
    })
}

/// The fixed-path operation `raw_path` names exactly, if any.
fn named_target(raw_path: &str) -> Option<Target> {
    CATALOG.iter().find_map(|spec| match spec.target {
        Target::Named(path) if path == raw_path => Some(spec.target),
        _ => None,
    })
}

/// The host without a `:port` (and without the brackets of an IPv6 literal's port form).
fn strip_port(host: &str) -> &str {
    if let Some(rest) = host.strip_prefix('[') {
        return rest.split_once(']').map_or(host, |(address, _)| address);
    }
    match host.rsplit_once(':') {
        Some((name, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => name,
        _ => host,
    }
}

/// Percent-decodes a path piece once, as UTF-8.
fn decode(raw: &str) -> Result<String> {
    let invalid = |_| {
        Error::new(
            ErrorCode::InvalidUri,
            "the request path is not valid percent-encoded UTF-8",
        )
    };
    String::from_utf8(percent_decode(raw).map_err(|_| invalid(()))?).map_err(|_| invalid(()))
}

#[cfg(test)]
mod tests {
    use super::{Addressed, resolve};
    use tessari_s3_core::dispatch::Target;
    use tessari_s3_types::ErrorCode;

    fn domains() -> Vec<String> {
        vec!["s3.example.com".to_owned()]
    }

    fn shape(addressed: &Addressed) -> (Target, Option<&str>, Option<&str>) {
        let bucket = addressed.bucket.as_ref().map(|b| b.as_str());
        (
            addressed.target,
            bucket,
            addressed.key.as_ref().map(|k| k.as_str()),
        )
    }

    #[test]
    fn virtual_hosted_and_path_style_address_the_same_object() {
        let virtual_hosted =
            resolve("photos.s3.example.com:9100", "/a/b.jpg", &domains()).expect("resolves");
        let path_style =
            resolve("s3.example.com", "/photos/a/b.jpg", &domains()).expect("resolves");
        assert_eq!(
            shape(&virtual_hosted),
            (Target::Object, Some("photos"), Some("a/b.jpg"))
        );
        assert_eq!(shape(&virtual_hosted), shape(&path_style));
    }

    #[test]
    fn a_host_outside_the_domains_is_path_style_and_never_a_bucket() {
        let addressed = resolve("10.0.0.5:9100", "/photos", &domains()).expect("resolves");
        assert_eq!(shape(&addressed), (Target::Bucket, Some("photos"), None));
        assert_eq!(
            shape(&resolve("10.0.0.5", "/", &domains()).expect("resolves")),
            (Target::Service, None, None)
        );
    }

    #[test]
    fn the_key_is_decoded_once_and_never_normalised() {
        for (raw, key) in [
            ("/bkt/a//b", "a//b"),
            ("/bkt/a/./b", "a/./b"),
            ("/bkt/x%252F", "x%2F"),
            ("/bkt/a+b", "a+b"),
        ] {
            let addressed = resolve("localhost", raw, &[]).expect("resolves");
            assert_eq!(
                addressed.key.as_ref().map(|k| k.as_str()),
                Some(key),
                "{raw}"
            );
        }
    }

    #[test]
    fn a_trailing_slash_after_the_bucket_is_the_bucket() {
        assert_eq!(
            shape(&resolve("localhost", "/bkt/", &[]).expect("resolves")),
            (Target::Bucket, Some("bkt"), None)
        );
    }

    #[test]
    fn invalid_names_and_paths_are_refused_with_their_codes() {
        let code = |host: &str, path: &str| {
            resolve(host, path, &domains())
                .map(|_| ())
                .map_err(|e| e.code)
        };
        assert_eq!(
            code("localhost", "/Bad_Bucket/k"),
            Err(ErrorCode::InvalidBucketName)
        );
        assert_eq!(
            code("localhost", &format!("/bkt/{}", "k".repeat(1025))),
            Err(ErrorCode::KeyTooLongError)
        );
        assert_eq!(code("localhost", "/bkt/%ff"), Err(ErrorCode::InvalidUri));
        assert_eq!(code("localhost", "/bkt/%zz"), Err(ErrorCode::InvalidUri));
    }

    #[test]
    fn a_fixed_operation_path_is_not_a_bucket() {
        let addressed = resolve("localhost", "/WriteGetObjectResponse", &[]).expect("resolves");
        assert_eq!(addressed.target, Target::Named("/WriteGetObjectResponse"));
    }
}
