//! The request pipeline: address, dispatch, authenticate, then the operation — and the refusal for every operation
//! that has no handler.

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::Response;
use tessari_s3_core::auth::SignedRequest;
use tessari_s3_core::dispatch::{DispatchRequest, Method, dispatch, is_implemented};
use tessari_s3_types::ErrorCode;

use super::address::resolve;
use super::authenticate::authenticate;
use super::query::decode_query;
use super::response::{error_response, new_request_id};
use crate::state::ApiState;
use crate::{Error, Result};

/// Serves one S3 request. The body is not read before authentication, so no `100 Continue` is sent to a client
/// that has not proved who it is.
pub async fn handle(State(state): State<ApiState>, request: Request) -> Response<Body> {
    let request_id = new_request_id();
    let resource = request.uri().path().to_owned();
    let outcome = serve(&state, &request);
    let error = match outcome {
        Ok(never) => match never {},
        Err(error) => error,
    };
    tracing::info!(
        request_id = %request_id,
        method = %request.method(),
        code = error.code.as_str(),
        status = error.code.http_status(),
        "request refused"
    );
    error_response(&error, &resource, &request_id)
}

/// The pipeline up to the operation. Every operation is refused today, so success is uninhabited.
fn serve(state: &ApiState, request: &Request) -> Result<std::convert::Infallible> {
    let method = Method::parse(request.method().as_str()).ok_or_else(|| {
        Error::new(
            ErrorCode::MethodNotAllowed,
            "the method is not part of the S3 API",
        )
    })?;
    let mut headers = Vec::with_capacity(request.headers().len());
    for (name, value) in request.headers() {
        let value = value.to_str().map_err(|_| {
            Error::new(
                ErrorCode::InvalidArgument,
                "a header value is not visible ASCII",
            )
        })?;
        headers.push((name.as_str(), value));
    }
    let host = request
        .headers()
        .get("host")
        .and_then(|value| value.to_str().ok())
        .or_else(|| {
            request
                .uri()
                .authority()
                .map(|authority| authority.as_str())
        })
        .ok_or_else(|| Error::new(ErrorCode::InvalidRequest, "the Host header is required"))?;
    let raw_path = request.uri().path();
    let raw_query = request.uri().query().unwrap_or("");
    let addressed = resolve(host, raw_path, state.domains())?;
    let query = decode_query(raw_query)?;
    let query_refs: Vec<(&str, Option<&str>)> = query
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_deref()))
        .collect();
    let header_names: Vec<&str> = headers.iter().map(|(name, _)| *name).collect();
    let spec = dispatch(&DispatchRequest {
        method,
        target: addressed.target,
        query: &query_refs,
        header_names: &header_names,
    })?;
    let signed = SignedRequest {
        method: request.method().as_str(),
        raw_path,
        raw_query,
        headers: &headers,
    };
    authenticate(state, &signed)?;
    if is_implemented(spec.operation) {
        // An operation marked implemented with no handler wired here is a defect; refuse rather than guess.
        return Err(Error::new(
            ErrorCode::InternalError,
            "the operation has no handler",
        ));
    }
    Err(Error::new(
        ErrorCode::NotImplemented,
        format!("{} is not implemented", spec.name),
    ))
}
