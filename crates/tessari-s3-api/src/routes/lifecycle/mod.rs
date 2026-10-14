//! GetBucketLifecycleConfiguration, PutBucketLifecycleConfiguration and DeleteBucketLifecycle. A configuration is
//! stored whole or not at all; what this server does not do — transitions, tag and size filters — is refused rather
//! than accepted and ignored (S3 ref 08 M7).

use std::fmt::Write as _;

use axum::body::Body;
use axum::http::{Response, StatusCode};
use tessari_s3_core::lifecycle::{Expiration, validate};
use tessari_s3_types::ErrorCode;

use super::buckets::{existing, no_such_bucket};
use crate::pipeline::body::{CONTROL_BODY_LIMIT, read_verified};
use crate::pipeline::call::Call;
use crate::pipeline::response::{empty_response, xml_response};
use crate::xml::{S3_NAMESPACE, escape};
use crate::{Error, Result};

mod document;

/// `GET /{bucket}?lifecycle`.
pub(crate) async fn get(call: &Call<'_>) -> Result<Response<Body>> {
    let bucket = existing(call).await?;
    let Some(rules) = bucket.lifecycle else {
        return Err(Error::new(
            ErrorCode::NoSuchLifecycleConfiguration,
            "The lifecycle configuration does not exist",
        ));
    };
    let mut out = format!("<LifecycleConfiguration xmlns=\"{S3_NAMESPACE}\">");
    for rule in &rules {
        out.push_str("<Rule>");
        if !rule.id.is_empty() {
            let _ = write!(out, "<ID>{}</ID>", escape(&rule.id));
        }
        let _ = write!(
            out,
            "<Filter><Prefix>{}</Prefix></Filter><Status>{}</Status>",
            escape(&rule.prefix),
            if rule.enabled { "Enabled" } else { "Disabled" }
        );
        match &rule.expiration {
            Some(Expiration::Days(days)) => {
                let _ = write!(out, "<Expiration><Days>{days}</Days></Expiration>");
            }
            Some(Expiration::Date(at)) => {
                let _ = write!(
                    out,
                    "<Expiration><Date>{}</Date></Expiration>",
                    at.iso8601_millis()
                );
            }
            None if rule.expired_object_delete_marker => {
                out.push_str(
                    "<Expiration><ExpiredObjectDeleteMarker>true</ExpiredObjectDeleteMarker></Expiration>",
                );
            }
            None => {}
        }
        if let Some(noncurrent) = &rule.noncurrent {
            let _ = write!(
                out,
                "<NoncurrentVersionExpiration><NoncurrentDays>{}</NoncurrentDays>",
                noncurrent.days
            );
            if let Some(keep) = noncurrent.keep {
                let _ = write!(
                    out,
                    "<NewerNoncurrentVersions>{keep}</NewerNoncurrentVersions>"
                );
            }
            out.push_str("</NoncurrentVersionExpiration>");
        }
        if let Some(days) = rule.abort_after_days {
            let _ = write!(
                out,
                "<AbortIncompleteMultipartUpload><DaysAfterInitiation>{days}</DaysAfterInitiation>\
                 </AbortIncompleteMultipartUpload>"
            );
        }
        out.push_str("</Rule>");
    }
    out.push_str("</LifecycleConfiguration>");
    Ok(xml_response(StatusCode::OK, out))
}

/// `PUT /{bucket}?lifecycle` — the whole configuration replaces the one before it.
pub(crate) async fn put(call: &Call<'_>, body: Body) -> Result<Response<Body>> {
    let name = call.bucket()?;
    let document = read_verified(body, call.verified, CONTROL_BODY_LIMIT).await?;
    let rules = document::parse(&document)?;
    validate(&rules)
        .map_err(|refused| Error::new(ErrorCode::InvalidArgument, refused.to_string()))?;
    if !call
        .state
        .storage()
        .buckets()
        .set_lifecycle(name, Some(&rules))
        .await?
    {
        return Err(no_such_bucket());
    }
    Ok(empty_response(StatusCode::OK))
}

/// `DELETE /{bucket}?lifecycle`.
pub(crate) async fn delete(call: &Call<'_>) -> Result<Response<Body>> {
    if !call
        .state
        .storage()
        .buckets()
        .set_lifecycle(call.bucket()?, None)
        .await?
    {
        return Err(no_such_bucket());
    }
    Ok(empty_response(StatusCode::NO_CONTENT))
}
