//! GetBucketVersioning and PutBucketVersioning. A bucket starts never versioned and answers an empty configuration;
//! once enabled it moves only between `Enabled` and `Suspended`. MFA delete is not offered, and a configuration asking
//! for it is refused rather than half applied.

use axum::body::Body;
use axum::http::{Response, StatusCode};
use tessari_s3_core::versioning::Versioning;
use tessari_s3_types::ErrorCode;

use super::buckets::{existing, no_such_bucket};
use crate::pipeline::body::{CONTROL_BODY_LIMIT, read_verified};
use crate::pipeline::call::Call;
use crate::pipeline::response::{empty_response, xml_response};
use crate::xml::{S3_NAMESPACE, element_text};
use crate::{Error, Result};

const ROOT: &str = "VersioningConfiguration";

/// `GET /{bucket}?versioning` — `Status` only once the bucket has been versioned.
pub(crate) async fn get(call: &Call<'_>) -> Result<Response<Body>> {
    let bucket = existing(call).await?;
    let status = bucket.versioning.map_or(String::new(), |state| {
        format!("<Status>{}</Status>", state.as_str())
    });
    Ok(xml_response(
        StatusCode::OK,
        format!("<{ROOT} xmlns=\"{S3_NAMESPACE}\">{status}</{ROOT}>"),
    ))
}

/// `PUT /{bucket}?versioning` — `Status` must be exactly `Enabled` or `Suspended`.
pub(crate) async fn put(call: &Call<'_>, body: Body) -> Result<Response<Body>> {
    let name = call.bucket()?;
    let document = read_verified(body, call.verified, CONTROL_BODY_LIMIT).await?;
    let malformed = || {
        Error::new(
            ErrorCode::MalformedXml,
            "Status must be Enabled or Suspended",
        )
    };
    let state = element_text(&document, ROOT, "Status")?
        .as_deref()
        .and_then(Versioning::parse)
        .ok_or_else(malformed)?;
    if element_text(&document, ROOT, "MfaDelete")?.as_deref() == Some("Enabled") {
        return Err(Error::new(
            ErrorCode::NotImplemented,
            "MFA delete is not supported",
        ));
    }
    if !call
        .state
        .storage()
        .buckets()
        .set_versioning(name, state)
        .await?
    {
        return Err(no_such_bucket());
    }
    Ok(empty_response(StatusCode::OK))
}
