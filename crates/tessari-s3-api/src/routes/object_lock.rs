//! GetObjectLockConfiguration and PutObjectLockConfiguration (S3 ref 08 M5). Object Lock is turned on at CreateBucket
//! or here on a bucket whose versioning is enabled, and is never turned off; the configuration may name a default
//! retention — a mode and whole days or years — that a version written without one gets.

use axum::body::Body;
use axum::http::{Response, StatusCode};
use tessari_s3_core::object_lock::{Configuration, DefaultRetention, Mode, Period};
use tessari_s3_storage::buckets::LockSet;
use tessari_s3_types::ErrorCode;

use super::buckets::{existing, no_such_bucket};
use crate::pipeline::body::{CONTROL_BODY_LIMIT, read_verified};
use crate::pipeline::call::Call;
use crate::pipeline::response::{empty_response, xml_response};
use crate::xml::{S3_NAMESPACE, element_text};
use crate::{Error, Result};

const ROOT: &str = "ObjectLockConfiguration";

/// The configuration document a bucket answers with.
pub(crate) fn document(configuration: &Configuration) -> String {
    let rule = configuration.default.map_or(String::new(), |default| {
        let (name, count) = match default.period {
            Period::Days(days) => ("Days", days),
            Period::Years(years) => ("Years", years),
        };
        format!(
            "<Rule><DefaultRetention><Mode>{}</Mode><{name}>{count}</{name}></DefaultRetention></Rule>",
            default.mode.as_str()
        )
    });
    format!(
        "<{ROOT} xmlns=\"{S3_NAMESPACE}\"><ObjectLockEnabled>Enabled</ObjectLockEnabled>{rule}</{ROOT}>"
    )
}

/// The configuration `body` states; `MalformedXML` or `InvalidArgument` when it is not one S3 accepts.
pub(crate) fn parse(body: &[u8]) -> Result<Configuration> {
    let malformed = || {
        Error::new(
            ErrorCode::MalformedXml,
            "the Object Lock configuration is not the expected shape",
        )
    };
    if element_text(body, ROOT, "ObjectLockEnabled")?.as_deref() != Some("Enabled") {
        return Err(malformed());
    }
    let mode = element_text(body, ROOT, "Mode")?;
    let days = element_text(body, ROOT, "Days")?;
    let years = element_text(body, ROOT, "Years")?;
    if mode.is_none() && days.is_none() && years.is_none() {
        return Ok(Configuration::default());
    }
    let invalid = |message: &str| Error::new(ErrorCode::InvalidArgument, message.to_owned());
    let mode = mode
        .as_deref()
        .and_then(Mode::parse)
        .ok_or_else(|| invalid("a default retention names a Mode of GOVERNANCE or COMPLIANCE"))?;
    let count = |text: &str| {
        text.parse::<u16>()
            .map_err(|_| invalid("a retention period is a whole number"))
    };
    let period = match (days.as_deref(), years.as_deref()) {
        (Some(days), None) => Period::Days(count(days)?),
        (None, Some(years)) => Period::Years(count(years)?),
        _ => return Err(invalid("a default retention names Days or Years, not both")),
    };
    let default = DefaultRetention::new(mode, period)
        .ok_or_else(|| invalid("a default retention is 1 to 36500 days or 1 to 100 years"))?;
    Ok(Configuration {
        default: Some(default),
    })
}

/// `GET /{bucket}?object-lock`.
pub(crate) async fn get(call: &Call<'_>) -> Result<Response<Body>> {
    let configuration = existing(call).await?.object_lock.ok_or_else(|| {
        Error::new(
            ErrorCode::ObjectLockConfigurationNotFoundError,
            "Object Lock configuration does not exist for this bucket",
        )
    })?;
    Ok(xml_response(StatusCode::OK, document(&configuration)))
}

/// The body of an Object Lock request, which S3 takes only with its `Content-MD5` or a checksum, validated.
pub(crate) async fn checked_document(call: &Call<'_>, body: Body) -> Result<axum::body::Bytes> {
    let integrity = crate::routes::objects::headers::read_integrity(call.headers)?;
    if integrity.content_md5.is_none() && integrity.checksum.is_none() {
        return Err(Error::new(
            ErrorCode::InvalidRequest,
            "Missing required header for this request: Content-MD5 or x-amz-checksum-*",
        ));
    }
    let document = read_verified(body, call.verified, CONTROL_BODY_LIMIT).await?;
    let mut hashes = tessari_s3_core::objects::checksum::Hashes::new();
    hashes.update(&document);
    crate::routes::objects::put::check(&integrity, &hashes.finish(), None)?;
    Ok(document)
}

/// `PUT /{bucket}?object-lock`.
pub(crate) async fn put(call: &Call<'_>, body: Body) -> Result<Response<Body>> {
    let document = checked_document(call, body).await?;
    let configuration = parse(&document)?;
    crate::routes::buckets::resident(call).await?;
    match call
        .state
        .storage()
        .buckets()
        .set_object_lock(call.bucket()?, &configuration)
        .await?
    {
        LockSet::Done => Ok(empty_response(StatusCode::OK)),
        LockSet::NoSuchBucket => Err(no_such_bucket()),
        LockSet::NotVersioned => Err(Error::new(
            ErrorCode::InvalidBucketState,
            "Object Lock needs the bucket's versioning to be enabled",
        )),
    }
}

#[cfg(test)]
mod tests {
    use tessari_s3_core::object_lock::{Configuration, DefaultRetention, Mode, Period};
    use tessari_s3_types::ErrorCode;

    use super::{document, parse};

    #[test]
    fn a_configuration_reads_back_as_it_was_written() {
        for configuration in [
            Configuration::default(),
            Configuration {
                default: DefaultRetention::new(Mode::Governance, Period::Days(30)),
            },
            Configuration {
                default: DefaultRetention::new(Mode::Compliance, Period::Years(7)),
            },
        ] {
            assert_eq!(
                parse(document(&configuration).as_bytes()).expect("parses"),
                configuration
            );
        }
    }

    #[test]
    fn a_configuration_s3_would_not_accept_is_refused() {
        let wrap =
            |inner: &str| format!("<ObjectLockConfiguration>{inner}</ObjectLockConfiguration>");
        let rule = |inner: &str| {
            wrap(&format!(
                "<ObjectLockEnabled>Enabled</ObjectLockEnabled><Rule><DefaultRetention>{inner}</DefaultRetention></Rule>"
            ))
        };
        for (body, code) in [
            (wrap(""), ErrorCode::MalformedXml),
            (
                wrap("<ObjectLockEnabled>Disabled</ObjectLockEnabled>"),
                ErrorCode::MalformedXml,
            ),
            (rule("<Mode>GOVERNANCE</Mode>"), ErrorCode::InvalidArgument),
            (
                rule("<Mode>GOVERNANCE</Mode><Days>1</Days><Years>1</Years>"),
                ErrorCode::InvalidArgument,
            ),
            (
                rule("<Mode>governance</Mode><Days>1</Days>"),
                ErrorCode::InvalidArgument,
            ),
            (
                rule("<Mode>COMPLIANCE</Mode><Days>0</Days>"),
                ErrorCode::InvalidArgument,
            ),
            (
                rule("<Mode>COMPLIANCE</Mode><Years>101</Years>"),
                ErrorCode::InvalidArgument,
            ),
            (
                rule("<Mode>COMPLIANCE</Mode><Days>-1</Days>"),
                ErrorCode::InvalidArgument,
            ),
        ] {
            assert_eq!(
                parse(body.as_bytes()).expect_err(&body).code,
                code,
                "{body}"
            );
        }
    }
}
