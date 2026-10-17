//! The browser side of CORS (S3 user guide, "Troubleshooting CORS", retrieved 2026-10-17). A preflight carries no
//! credentials, so it is answered before authentication from the bucket's rules alone, and every refusal — no bucket,
//! no configuration, no matching rule — is the same 403, so a preflight learns nothing about what exists.

use axum::body::Body;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Response, StatusCode, header};
use tessari_s3_core::cors::{Rule, actual, allowed_origin};
use tessari_s3_types::{BucketName, ErrorCode};

use crate::pipeline::response::empty_response;
use crate::state::ApiState;
use crate::{Error, Result};

fn forbidden() -> Error {
    Error::new(
        ErrorCode::AccessForbidden,
        "CORSResponse: This CORS request is not allowed.",
    )
}

fn text<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn put(response: &mut Response<Body>, name: HeaderName, value: &str) {
    if let Ok(value) = HeaderValue::from_str(value) {
        response.headers_mut().insert(name, value);
    }
}

/// The headers every allowed cross-origin answer carries: who may read it, which headers it may show, and that it
/// varies by origin.
fn allow(response: &mut Response<Body>, rule: &Rule, origin: &str) {
    let allowed = allowed_origin(rule, origin);
    put(response, header::ACCESS_CONTROL_ALLOW_ORIGIN, allowed);
    if allowed != "*" {
        put(response, header::ACCESS_CONTROL_ALLOW_CREDENTIALS, "true");
    }
    if !rule.expose.is_empty() {
        put(
            response,
            header::ACCESS_CONTROL_EXPOSE_HEADERS,
            &rule.expose.join(", "),
        );
    }
    response
        .headers_mut()
        .append(header::VARY, HeaderValue::from_static("Origin"));
}

/// `OPTIONS /{bucket}[/{key}]`: the first rule matching the origin, the requested method and every requested header.
///
/// # Errors
/// `BadRequest` without an Origin or a requested method; `AccessForbidden` when nothing allows it; the metadata store's
/// outage.
pub(crate) async fn preflight(
    state: &ApiState,
    bucket: Option<&BucketName>,
    headers: &HeaderMap,
) -> Result<Response<Body>> {
    let (Some(origin), Some(method)) = (
        text(headers, "origin"),
        text(headers, "access-control-request-method"),
    ) else {
        return Err(Error::new(
            ErrorCode::BadRequest,
            "Insufficient information. Origin and Access-Control-Request-Method are needed.",
        ));
    };
    let asked: Vec<String> = text(headers, "access-control-request-headers")
        .map(|list| {
            list.split(',')
                .map(|name| name.trim().to_ascii_lowercase())
                .filter(|name| !name.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let Some(bucket) = bucket else {
        return Err(forbidden());
    };
    let rules = state
        .storage()
        .buckets()
        .get(bucket)
        .await?
        .and_then(|bucket| bucket.cors)
        .ok_or_else(forbidden)?;
    let asked_refs: Vec<&str> = asked.iter().map(String::as_str).collect();
    let rule = tessari_s3_core::cors::preflight(&rules, origin, method, &asked_refs)
        .ok_or_else(forbidden)?;
    let mut response = empty_response(StatusCode::OK);
    allow(&mut response, rule, origin);
    let methods: Vec<&str> = rule.methods.iter().map(|method| method.as_str()).collect();
    put(
        &mut response,
        header::ACCESS_CONTROL_ALLOW_METHODS,
        &methods.join(", "),
    );
    if !asked.is_empty() {
        put(
            &mut response,
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            &asked.join(", "),
        );
    }
    if let Some(seconds) = rule.max_age {
        put(
            &mut response,
            header::ACCESS_CONTROL_MAX_AGE,
            &seconds.to_string(),
        );
    }
    for varies in [
        "Access-Control-Request-Headers",
        "Access-Control-Request-Method",
    ] {
        response
            .headers_mut()
            .append(header::VARY, HeaderValue::from_static(varies));
    }
    Ok(response)
}

/// Adds to `response` what the first rule matching `origin` and `method` allows; a bucket that cannot be read, or no
/// matching rule, adds nothing — and a browser then keeps the answer from the script, which is the refusal.
pub(crate) async fn answer_origin(
    state: &ApiState,
    bucket: &BucketName,
    origin: &str,
    method: &str,
    response: &mut Response<Body>,
) {
    let rules = match state.storage().buckets().get(bucket).await {
        Ok(found) => found.and_then(|bucket| bucket.cors),
        Err(error) => {
            tracing::debug!(%error, "CORS rules not read; no CORS headers added");
            None
        }
    };
    if let Some(rule) = rules
        .as_deref()
        .and_then(|rules| actual(rules, origin, method))
    {
        allow(response, rule, origin);
    }
}
