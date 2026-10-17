//! GetObjectRetention, PutObjectRetention, GetObjectLegalHold and PutObjectLegalHold (S3 ref 08 M5): one version's
//! retention and legal hold, named by `versionId` or the key's current version, in a bucket with Object Lock. A
//! retention may be extended by whoever may write; shortening or removing one that still keeps the version is refused,
//! except a GOVERNANCE one by who administers the bucket and asks with `x-amz-bypass-governance-retention: true`.

use axum::body::Body;
use axum::http::{Response, StatusCode};
use tessari_s3_core::object_lock::{LegalHold, Mode};
use tessari_s3_storage::objects::LockOutcome;
use tessari_s3_types::{ErrorCode, ObjectKey, Timestamp};

use super::buckets::{existing, no_such_bucket};
use super::object_lock::checked_document;
use crate::pipeline::authorize::may_bypass;
use crate::pipeline::call::Call;
use crate::pipeline::response::{empty_response, xml_response};
use crate::xml::{S3_NAMESPACE, element_text};
use crate::{Error, Result};

/// Whether the request asks to bypass governance retention.
pub(crate) fn bypass_asked(call: &Call<'_>) -> bool {
    call.headers
        .get("x-amz-bypass-governance-retention")
        .is_some_and(|value| value.as_bytes().eq_ignore_ascii_case(b"true"))
}

/// `AccessDenied`: Object Lock keeps the version.
pub(crate) fn locked() -> Error {
    Error::new(
        ErrorCode::AccessDenied,
        "the version is protected by Object Lock",
    )
}

/// The bucket's Object Lock must be on for these requests.
pub(crate) async fn lock_bucket(call: &Call<'_>) -> Result<()> {
    if existing(call).await?.object_lock.is_none() {
        return Err(missing_lock());
    }
    Ok(())
}

fn missing_lock() -> Error {
    Error::new(
        ErrorCode::InvalidRequest,
        "Bucket is missing Object Lock Configuration",
    )
}

/// A write's Object Lock headers are taken only by a bucket with Object Lock, and a retention — asked for, or the
/// bucket's default — only with the bytes' `Content-MD5` or a checksum, so a locked version is provably what the
/// client sent (S3 ref 08 M5). A write that proves its bytes and asks for nothing reads no bucket here.
pub(crate) async fn admit_lock(
    call: &Call<'_>,
    asked: &crate::routes::objects::headers::LockAsked,
    integrity: &crate::routes::objects::headers::Integrity,
) -> Result<()> {
    let digested = integrity.content_md5.is_some() || integrity.checksum.is_some();
    if !asked.asked && digested {
        return Ok(());
    }
    let lock = existing(call).await?.object_lock;
    if asked.asked && lock.is_none() {
        return Err(missing_lock());
    }
    let retained = asked.retention.is_some()
        || lock.is_some_and(|configuration| configuration.default.is_some());
    if retained && !digested {
        return Err(Error::new(
            ErrorCode::InvalidRequest,
            "Content-MD5 or x-amz-checksum-* is required for an object written with a retention",
        ));
    }
    Ok(())
}

fn answered(outcome: LockOutcome) -> Result<(String, tessari_s3_storage::objects::VersionLock)> {
    match outcome {
        LockOutcome::Version { version_id, lock } => Ok((version_id, lock)),
        LockOutcome::NoSuchBucket => Err(no_such_bucket()),
        LockOutcome::NoSuchKey => Err(Error::new(
            ErrorCode::NoSuchKey,
            "the specified key does not exist",
        )),
        LockOutcome::NoSuchVersion => Err(Error::new(
            ErrorCode::NoSuchVersion,
            "the specified version does not exist",
        )),
        LockOutcome::Malformed => Err(Error::new(
            ErrorCode::InvalidArgument,
            "Invalid version id specified",
        )),
        LockOutcome::Marker => Err(Error::new(
            ErrorCode::MethodNotAllowed,
            "the specified method is not allowed against a delete marker",
        )),
        LockOutcome::Locked => Err(locked()),
    }
}

fn with_version(response: Response<Body>, version_id: &str) -> Response<Body> {
    crate::pipeline::response::with_version(response, Some(version_id))
}

/// `GET /{bucket}/{key}?retention`.
pub(crate) async fn get_retention(call: &Call<'_>, key: &ObjectKey) -> Result<Response<Body>> {
    lock_bucket(call).await?;
    let outcome = call
        .state
        .storage()
        .objects()
        .version_lock(call.bucket()?, key, call.query_value("versionId"))
        .await?;
    let (version_id, lock) = answered(outcome)?;
    let Some((mode, until)) = lock.retention else {
        return Err(Error::new(
            ErrorCode::ObjectLockConfigurationNotFoundError,
            "the object has no retention",
        ));
    };
    let body = format!(
        "<Retention xmlns=\"{S3_NAMESPACE}\"><Mode>{}</Mode><RetainUntilDate>{}</RetainUntilDate></Retention>",
        mode.as_str(),
        until.iso8601_millis()
    );
    Ok(with_version(
        xml_response(StatusCode::OK, body),
        &version_id,
    ))
}

/// `PUT /{bucket}/{key}?retention`.
pub(crate) async fn put_retention(
    call: &Call<'_>,
    key: &ObjectKey,
    body: Body,
) -> Result<Response<Body>> {
    let document = checked_document(call, body).await?;
    let mode = element_text(&document, "Retention", "Mode")?;
    let until = element_text(&document, "Retention", "RetainUntilDate")?;
    let invalid = |message: &str| Error::new(ErrorCode::InvalidArgument, message.to_owned());
    let retention = match (mode.as_deref(), until.as_deref()) {
        (None, None) => None,
        (Some(mode), Some(until)) => {
            let mode =
                Mode::parse(mode).ok_or_else(|| invalid("Mode is GOVERNANCE or COMPLIANCE"))?;
            let until = Timestamp::parse_iso8601(until)
                .ok_or_else(|| invalid("RetainUntilDate is an ISO 8601 instant"))?;
            if until <= Timestamp::now() {
                return Err(invalid("The retain until date must be in the future"));
            }
            Some((mode, until))
        }
        _ => {
            return Err(invalid(
                "a retention names both a Mode and a RetainUntilDate",
            ));
        }
    };
    crate::routes::buckets::resident(call).await?;
    lock_bucket(call).await?;
    let bucket = call.bucket()?;
    // Bypassing governance retention is what administering the bucket allows, and only when asked for.
    let bypass = bypass_asked(call)
        && may_bypass(call.state, call.principal, bucket, key, call.context).await?;
    if bypass_asked(call) && !bypass {
        return Err(Error::new(
            ErrorCode::AccessDenied,
            "bypassing governance retention needs who administers the bucket",
        ));
    }
    let outcome = call
        .state
        .storage()
        .objects()
        .set_retention(
            bucket,
            key,
            call.query_value("versionId"),
            retention,
            bypass,
        )
        .await?;
    let (version_id, _) = answered(outcome)?;
    Ok(with_version(empty_response(StatusCode::OK), &version_id))
}

/// `GET /{bucket}/{key}?legal-hold`.
pub(crate) async fn get_legal_hold(call: &Call<'_>, key: &ObjectKey) -> Result<Response<Body>> {
    lock_bucket(call).await?;
    let outcome = call
        .state
        .storage()
        .objects()
        .version_lock(call.bucket()?, key, call.query_value("versionId"))
        .await?;
    let (version_id, lock) = answered(outcome)?;
    let status = if lock.legal_hold {
        LegalHold::On
    } else {
        LegalHold::Off
    };
    let body = format!(
        "<LegalHold xmlns=\"{S3_NAMESPACE}\"><Status>{}</Status></LegalHold>",
        status.as_str()
    );
    Ok(with_version(
        xml_response(StatusCode::OK, body),
        &version_id,
    ))
}

/// `PUT /{bucket}/{key}?legal-hold`.
pub(crate) async fn put_legal_hold(
    call: &Call<'_>,
    key: &ObjectKey,
    body: Body,
) -> Result<Response<Body>> {
    let document = checked_document(call, body).await?;
    let status = element_text(&document, "LegalHold", "Status")?
        .as_deref()
        .and_then(LegalHold::parse)
        .ok_or_else(|| Error::new(ErrorCode::MalformedXml, "Status is ON or OFF"))?;
    crate::routes::buckets::resident(call).await?;
    lock_bucket(call).await?;
    let outcome = call
        .state
        .storage()
        .objects()
        .set_legal_hold(
            call.bucket()?,
            key,
            call.query_value("versionId"),
            status == LegalHold::On,
        )
        .await?;
    let (version_id, _) = answered(outcome)?;
    Ok(with_version(empty_response(StatusCode::OK), &version_id))
}
