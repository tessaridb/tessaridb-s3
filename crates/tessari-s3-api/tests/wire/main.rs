//! The HTTP surface end to end, through the router with no network: AWS's published requests, every catalog
//! operation signed and sent, and the refusals a client can see.

#![cfg(test)]

mod operations;
mod refusals;
mod signer;

use axum::body::{Body, to_bytes};
use axum::http::{Request, Response};
use tessari_s3_api::{ApiState, router};
use tessari_s3_infrastructure::S3Config;
use tower::ServiceExt;

/// The published examples' credentials, region, and signing time (2013-05-24T00:00:00Z).
pub(crate) const ACCESS_KEY: &str = "AKIAIOSFODNN7EXAMPLE";
pub(crate) const SECRET: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
pub(crate) const NOW: i64 = 1_369_353_600;
pub(crate) const AMZ_DATE: &str = "20130524T000000Z";

/// A state signing for us-east-1 with `s3.amazonaws.com` as the virtual-hosting domain, at `now`.
pub(crate) fn state(now: i64, max_inflight: usize) -> ApiState {
    let vars = [
        ("TESSARIDB_S3_ROOT_ACCESS_KEY", ACCESS_KEY),
        ("TESSARIDB_S3_ROOT_SECRET_KEY", SECRET),
        ("TESSARIDB_S3_DOMAINS", "s3.amazonaws.com"),
    ];
    let config = S3Config::from_lookup(|key| {
        vars.iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| (*v).to_owned())
    })
    .expect("test configuration loads");
    ApiState::with_limit(&config, std::sync::Arc::new(move || now), max_inflight)
}

/// What a client sees: status, the request-id header, the `<Code>` and the body.
pub(crate) struct Seen {
    pub(crate) status: u16,
    pub(crate) request_id: Option<String>,
    pub(crate) code: Option<String>,
    pub(crate) body: String,
}

/// Sends `request` through a fresh router built on `state`.
pub(crate) async fn send(state: ApiState, request: Request<Body>) -> Seen {
    let response: Response<Body> = router(state)
        .oneshot(request)
        .await
        .expect("the router is infallible");
    let status = response.status().as_u16();
    let request_id = response
        .headers()
        .get("x-amz-request-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let body = String::from_utf8(
        to_bytes(response.into_body(), 1 << 20)
            .await
            .expect("body")
            .to_vec(),
    )
    .expect("utf-8 body");
    let code = body
        .split_once("<Code>")
        .and_then(|(_, rest)| rest.split_once("</Code>"))
        .map(|(c, _)| c.to_owned());
    Seen {
        status,
        request_id,
        code,
        body,
    }
}

/// Builds a request from a target and header pairs.
pub(crate) fn request(method: &str, target: &str, headers: &[(String, String)]) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(target);
    for (name, value) in headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    builder.body(Body::empty()).expect("valid request")
}
