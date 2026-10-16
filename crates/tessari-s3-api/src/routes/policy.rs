//! Bucket policies (S3 ref 04 M4): `PUT`, `GET` and `DELETE /{bucket}?policy`. A document is validated by the one
//! evaluator's parser before it is stored, so a stored policy is always one this server evaluates as written.

use axum::body::Body;
use axum::http::{Response, StatusCode, header};
use tessari_s3_core::authz::policy::BucketPolicy;
use tessari_s3_types::ErrorCode;

use super::buckets::{existing, no_such_bucket, resident};
use crate::pipeline::body::{CONTROL_BODY_LIMIT, read_verified};
use crate::pipeline::call::Call;
use crate::pipeline::response::empty_response;
use crate::{Error, Result};

/// `PUT /{bucket}?policy`: the document, validated against this bucket and its space.
pub(crate) async fn put(call: &Call<'_>, body: Body) -> Result<Response<Body>> {
    let document = read_verified(body, call.verified, CONTROL_BODY_LIMIT).await?;
    let bucket = existing(call).await?;
    let malformed = |message: String| Error::new(ErrorCode::MalformedPolicy, message);
    let text = std::str::from_utf8(&document)
        .map_err(|_| malformed("the policy is not JSON".to_owned()))?;
    BucketPolicy::parse(text, bucket.name.as_str(), &bucket.space)
        .map_err(|error| malformed(error.to_string()))?;
    resident(call).await?;
    set(call, Some(text)).await
}

/// `GET /{bucket}?policy`: the document as it was written.
pub(crate) async fn get(call: &Call<'_>) -> Result<Response<Body>> {
    let text = existing(call).await?.policy.ok_or_else(|| {
        Error::new(
            ErrorCode::NoSuchBucketPolicy,
            "The bucket policy does not exist",
        )
    })?;
    let mut response = Response::new(Body::from(text));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/json"),
    );
    Ok(response)
}

/// `DELETE /{bucket}?policy`; removing a policy that is not there succeeds.
pub(crate) async fn delete(call: &Call<'_>) -> Result<Response<Body>> {
    set(call, None).await
}

async fn set(call: &Call<'_>, text: Option<&str>) -> Result<Response<Body>> {
    if call
        .state
        .storage()
        .buckets()
        .set_policy(call.bucket()?, text)
        .await?
    {
        Ok(empty_response(StatusCode::NO_CONTENT))
    } else {
        Err(no_such_bucket())
    }
}
