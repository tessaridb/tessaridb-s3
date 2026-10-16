//! A browser's form upload, `POST /{bucket}` with `multipart/form-data` (S3 ref 03 M7). The fields come first and are
//! read whole (bounded); the file is streamed into the same write PutObject makes. A form with a `policy` is
//! authenticated by its signature over that policy and must satisfy every condition on the expanded form; a form
//! without one is anonymous. Either way the upload is then authorized exactly as the principal's PutObject of the key.

use std::sync::atomic::Ordering;

use axum::body::Body;
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Response, StatusCode, header};
use tessari_s3_core::auth::{PayloadHash, PostPolicy, PostPolicyError, Verified, verify_post};
use tessari_s3_core::authz::Principal;
use tessari_s3_core::dispatch::{Method, Operation, Target};
use tessari_s3_types::{ErrorCode, ObjectKey};

use super::address::Addressed;
use super::authorize::authorize_operation;
use super::call::Call;
use super::fence::writable;
use super::trail::Trail;
use crate::state::ApiState;
use crate::xml::{S3_NAMESPACE, escape};
use crate::{Error, Result, routes};

mod reader;

/// Form fields that sign the form rather than describe the object.
const SIGNING: [&str; 5] = [
    "x-amz-algorithm",
    "x-amz-credential",
    "x-amz-date",
    "x-amz-signature",
    "x-amz-security-token",
];

/// Whether this request is a form upload: a POST to a bucket, no subresource, a multipart form body.
pub(super) fn is_form_upload(parts: &Parts, addressed: &Addressed) -> bool {
    parts.method == axum::http::Method::POST
        && addressed.bucket.is_some()
        && addressed.key.is_none()
        && parts.uri.query().is_none_or(str::is_empty)
        && parts
            .headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(reader::boundary)
            .is_some()
}

fn field<'a>(fields: &'a [(String, String)], name: &str) -> Option<&'a str> {
    fields
        .iter()
        .find(|(field, _)| field == name)
        .map(|(_, value)| value.as_str())
}

fn required<'a>(fields: &'a [(String, String)], name: &str) -> Result<&'a str> {
    field(fields, name).ok_or_else(|| {
        Error::new(
            ErrorCode::InvalidArgument,
            format!("Bucket POST must contain a field named '{name}'"),
        )
    })
}

fn policy_refusal(error: &PostPolicyError) -> Error {
    match error {
        PostPolicyError::Malformed(_) => {
            Error::new(ErrorCode::InvalidPolicyDocument, error.to_string())
        }
        _ => Error::new(ErrorCode::AccessDenied, error.to_string()),
    }
}

/// Who signed the form and the size range its policy allows; a form without a policy is anonymous.
async fn authenticate(
    state: &ApiState,
    bucket: &str,
    fields: &[(String, String)],
) -> Result<(Verified, Principal, Option<(u64, u64)>)> {
    let Some(encoded) = field(fields, "policy") else {
        let verified = Verified::anonymous(PayloadHash::Unsigned).map_err(Error::from)?;
        return Ok((verified, Principal::Anonymous, None));
    };
    let credential = required(fields, "x-amz-credential")?;
    let access_key = credential.split('/').next().unwrap_or_default();
    let known = state.credential_for(access_key).await?;
    let verified = verify_post(
        encoded,
        required(fields, "x-amz-algorithm")?,
        credential,
        required(fields, "x-amz-date")?,
        required(fields, "x-amz-signature")?,
        &known.secret,
        state.region(),
    )
    .map_err(Error::from)?;
    let range = PostPolicy::parse(encoded)
        .map_err(|error| policy_refusal(&error))?
        .check(bucket, fields, state.now())
        .map_err(|error| policy_refusal(&error))?;
    Ok((verified, known.principal, range))
}

/// The headers the object is written with: the form's content and `x-amz-*` fields, its ACL, its length.
fn object_headers(fields: &[(String, String)], length: u64) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    for (name, value) in fields {
        let target = match name.as_str() {
            "acl" => "x-amz-acl",
            "content-type"
            | "cache-control"
            | "content-disposition"
            | "content-encoding"
            | "content-language"
            | "expires" => name.as_str(),
            other if other.starts_with("x-amz-") && !SIGNING.contains(&other) => other,
            _ => continue,
        };
        let (Ok(target), Ok(value)) = (
            HeaderName::from_bytes(target.as_bytes()),
            HeaderValue::from_str(value),
        ) else {
            return Err(Error::new(
                ErrorCode::InvalidArgument,
                format!("the field {name} cannot be stored as a header"),
            ));
        };
        headers.insert(target, value);
    }
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from(length));
    Ok(headers)
}

/// Serves a form upload to the bucket `addressed` names.
///
/// # Errors
/// `MissingContentLength`, `MalformedPOSTRequest`, `InvalidPolicyDocument`, the signature refusals, `AccessDenied`
/// for a policy the form leaves or an upload the principal may not make, `EntityTooSmall`/`EntityTooLarge` outside
/// the policy's range, and what PutObject refuses.
pub(super) async fn post(
    state: &ApiState,
    parts: &Parts,
    addressed: &Addressed,
    body: Body,
    trail: &mut Trail,
) -> Result<Response<Body>> {
    let bucket = addressed
        .bucket
        .clone()
        .ok_or_else(|| reader::malformed("a form upload names its bucket"))?;
    let boundary = parts
        .headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(reader::boundary)
        .ok_or_else(|| reader::malformed("the form has no boundary"))?;
    let content_length = parts
        .headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| {
            Error::new(
                ErrorCode::MissingContentLength,
                "a form upload declares its length",
            )
        })?;
    let mut form = reader::read(body, &boundary, content_length).await?;
    let expanded = required(&form.fields, "key")?.replace("${filename}", &form.filename);
    if let Some((_, key)) = form.fields.iter_mut().find(|(name, _)| name == "key") {
        key.clone_from(&expanded);
    }
    let (verified, principal, range) = authenticate(state, bucket.as_str(), &form.fields).await?;
    if trail.logged {
        trail.principal = Some(if verified.is_anonymous() {
            "anonymous".to_owned()
        } else {
            verified.access_key().to_owned()
        });
    }
    if let Some((min, max)) = range {
        if form.length < min {
            return Err(Error::new(
                ErrorCode::EntityTooSmall,
                "the file is smaller than the policy allows",
            ));
        }
        if form.length > max {
            return Err(Error::new(
                ErrorCode::EntityTooLarge,
                "the file is larger than the policy allows",
            ));
        }
    }
    let key = ObjectKey::new(&expanded).map_err(|_| {
        Error::new(
            ErrorCode::InvalidArgument,
            "the key field is not a valid key",
        )
    })?;
    let addressed = Addressed {
        target: Target::Object,
        bucket: Some(bucket.clone()),
        key: Some(key.clone()),
    };
    let headers = object_headers(&form.fields, form.length)?;
    let secure = parts.extensions.get::<crate::OverTls>().is_some();
    let mut context =
        super::handler::context(state, parts, &[], Operation::PutObject, &verified, secure);
    context.server_side_encryption =
        field(&form.fields, "x-amz-server-side-encryption").map(str::to_owned);
    authorize_operation(
        state,
        Operation::PutObject,
        &principal,
        &addressed,
        &headers,
        false,
        &context,
    )
    .await?;
    writable(state, Method::Post, false).await?;
    let call = Call {
        state,
        addressed: &addressed,
        query: &[],
        verified: &verified,
        principal: &principal,
        headers: &headers,
        secure,
        context: &context,
    };
    let file = std::mem::replace(&mut form.file, Body::empty());
    let stored = match routes::objects::put(&call, &key, file).await {
        Ok(stored) => stored,
        Err(_) if form.broken.load(Ordering::Acquire) => {
            return Err(reader::malformed(
                "the form does not end where its length says",
            ));
        }
        Err(error) => return Err(error),
    };
    answer(&form.fields, bucket.as_str(), key.as_str(), &stored)
}

/// The answer the form asked for: a redirect, a `201` document, or an empty `200`/`204` (the default).
fn answer(
    fields: &[(String, String)],
    bucket: &str,
    key: &str,
    stored: &Response<Body>,
) -> Result<Response<Body>> {
    let etag = stored
        .headers()
        .get(header::ETAG)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let mut response = if let Some(target) = field(fields, "success_action_redirect") {
        let mut query = String::new();
        for (name, value) in [("bucket", bucket), ("key", key), ("etag", etag.as_str())] {
            query.push(if query.is_empty() {
                if target.contains('?') { '&' } else { '?' }
            } else {
                '&'
            });
            query.push_str(name);
            query.push('=');
            tessari_s3_core::auth::uri_encode(value.as_bytes(), false, &mut query);
        }
        let mut redirect = super::response::empty_response(StatusCode::SEE_OTHER);
        let location = HeaderValue::from_str(&format!("{target}{query}")).map_err(|_| {
            Error::new(
                ErrorCode::InvalidArgument,
                "success_action_redirect is not a URL",
            )
        })?;
        redirect.headers_mut().insert(header::LOCATION, location);
        redirect
    } else {
        match field(fields, "success_action_status") {
            Some("201") => super::response::xml_response(
                StatusCode::CREATED,
                format!(
                    "<PostResponse xmlns=\"{S3_NAMESPACE}\"><Location>/{bucket}/{key}</Location><Bucket>{bucket}</Bucket><Key>{key}</Key><ETag>{etag}</ETag></PostResponse>",
                    bucket = escape(bucket),
                    key = escape(key),
                    etag = escape(&etag),
                ),
            ),
            Some("200") => super::response::empty_response(StatusCode::OK),
            _ => super::response::empty_response(StatusCode::NO_CONTENT),
        }
    };
    if let Ok(value) = HeaderValue::from_str(&etag) {
        response.headers_mut().insert(header::ETAG, value);
    }
    Ok(response)
}
