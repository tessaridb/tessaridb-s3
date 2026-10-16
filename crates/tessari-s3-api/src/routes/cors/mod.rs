//! GetBucketCors, PutBucketCors and DeleteBucketCors, and the browser side of CORS: the unauthenticated preflight and
//! the headers an actual request's answer carries.

use std::fmt::Write as _;

use axum::body::Body;
use axum::http::{Response, StatusCode};
use tessari_s3_core::cors::validate;
use tessari_s3_types::ErrorCode;

use super::buckets::{existing, no_such_bucket, resident};
use crate::pipeline::body::{CONTROL_BODY_LIMIT, read_verified};
use crate::pipeline::call::Call;
use crate::pipeline::response::{empty_response, xml_response};
use crate::xml::{S3_NAMESPACE, escape};
use crate::{Error, Result};

mod browser;
mod document;

pub(crate) use browser::{answer_origin, preflight};

/// `GET /{bucket}?cors`.
pub(crate) async fn get(call: &Call<'_>) -> Result<Response<Body>> {
    let rules = existing(call).await?.cors.ok_or_else(|| {
        Error::new(
            ErrorCode::NoSuchCorsConfiguration,
            "The CORS configuration does not exist",
        )
    })?;
    let mut out = format!("<CORSConfiguration xmlns=\"{S3_NAMESPACE}\">");
    for rule in &rules {
        out.push_str("<CORSRule>");
        if let Some(id) = &rule.id {
            let _ = write!(out, "<ID>{}</ID>", escape(id));
        }
        for (name, values) in [
            ("AllowedOrigin", &rule.origins),
            ("AllowedHeader", &rule.headers),
            ("ExposeHeader", &rule.expose),
        ] {
            for value in values {
                let _ = write!(out, "<{name}>{}</{name}>", escape(value));
            }
        }
        for method in &rule.methods {
            let _ = write!(out, "<AllowedMethod>{}</AllowedMethod>", method.as_str());
        }
        if let Some(seconds) = rule.max_age {
            let _ = write!(out, "<MaxAgeSeconds>{seconds}</MaxAgeSeconds>");
        }
        out.push_str("</CORSRule>");
    }
    out.push_str("</CORSConfiguration>");
    Ok(xml_response(StatusCode::OK, out))
}

/// `PUT /{bucket}?cors` — the whole configuration replaces the one before it.
pub(crate) async fn put(call: &Call<'_>, body: Body) -> Result<Response<Body>> {
    let document = read_verified(body, call.verified, CONTROL_BODY_LIMIT).await?;
    let rules = document::parse(&document)?;
    validate(&rules)
        .map_err(|refused| Error::new(ErrorCode::InvalidRequest, refused.to_string()))?;
    resident(call).await?;
    set(call, Some(&rules), StatusCode::OK).await
}

/// `DELETE /{bucket}?cors`.
pub(crate) async fn delete(call: &Call<'_>) -> Result<Response<Body>> {
    set(call, None, StatusCode::NO_CONTENT).await
}

async fn set(
    call: &Call<'_>,
    rules: Option<&[tessari_s3_core::cors::Rule]>,
    status: StatusCode,
) -> Result<Response<Body>> {
    if call
        .state
        .storage()
        .buckets()
        .set_cors(call.bucket()?, rules)
        .await?
    {
        Ok(empty_response(status))
    } else {
        Err(no_such_bucket())
    }
}
