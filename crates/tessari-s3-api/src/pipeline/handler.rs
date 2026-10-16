//! The request pipeline: address, dispatch, authenticate, authorize, then the operation — or the refusal for an
//! operation with no handler.

use std::time::Instant;

use axum::body::{Body, HttpBody};
use axum::extract::{Request, State};
use axum::http::Response;
use tessari_s3_core::auth::SignedRequest;
use tessari_s3_core::authz::Principal;
use tessari_s3_core::authz::policy::Context;
use tessari_s3_core::dispatch::{DispatchRequest, Method, dispatch, is_implemented};
use tessari_s3_observability::{AuthRefusal, Served};
use tessari_s3_types::ErrorCode;

use super::address::resolve;
use super::authenticate::authenticate;
use super::authorize::{authorize_operation, may_list};
use super::call::Call;
use super::fence::writable;
use super::query::decode_query;
use super::response::{error_response, new_request_id, with_request_id};
use super::trail::{Answered, Trail, client, log};
use crate::state::ApiState;
use crate::{Error, Result, routes};

/// Serves one S3 request. The body is not read before authentication, so no `100 Continue` is sent to a client
/// that has not proved who it is. Each request is counted in the node's metrics by the operation it was dispatched
/// to (`Unknown` when it was refused before dispatch named one), its status and its error code.
pub async fn handle(State(state): State<ApiState>, request: Request) -> Response<Body> {
    let arrived = Instant::now();
    let request_id = new_request_id();
    let resource = request.uri().path().to_owned();
    let method = request.method().clone();
    let sent = content_length(request.headers());
    let at_secs = state.now();
    let mut trail = Trail::new(&state);
    let client = if trail.logged {
        client(request.extensions())
    } else {
        None
    };
    let (response, code) = match serve(state.clone(), request, &mut trail).await {
        Ok(response) => {
            tracing::info!(request_id = %request_id, method = %method, status = response.status().as_u16(), "served");
            (with_request_id(response, &request_id), "None")
        }
        Err(error) => {
            tracing::info!(
                request_id = %request_id,
                method = %method,
                code = error.code.as_str(),
                status = error.code.http_status(),
                "refused"
            );
            (
                error_response(&error, &resource, &request_id),
                error.code.as_str(),
            )
        }
    };
    // A streamed body has no exact size; its length is the header it was answered with.
    let answered =
        content_length(response.headers()).max(response.body().size_hint().exact().unwrap_or(0));
    let elapsed = arrived.elapsed();
    let bytes = sent.max(answered);
    let status = response.status().as_u16();
    state.metrics().served(&Served {
        operation: trail.operation,
        status,
        code,
        bytes,
        seconds: elapsed.as_secs_f64(),
    });
    if trail.logged {
        log(
            &state,
            trail,
            Answered {
                request_id,
                at_secs,
                method: method.to_string(),
                client,
                status,
                code,
                bytes,
                millis: u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
            },
        );
    }
    response
}

/// The declared body length in `headers`; `0` when they declare none.
fn content_length(headers: &axum::http::HeaderMap) -> u64 {
    headers
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

/// The pipeline up to and including the operation; `trail` learns the operation once dispatch has named it, and the
/// bucket, key and signer while the access log is on.
async fn serve(state: ApiState, request: Request, trail: &mut Trail) -> Result<Response<Body>> {
    let (parts, body) = request.into_parts();
    let method = Method::parse(parts.method.as_str()).ok_or_else(|| {
        Error::new(
            ErrorCode::MethodNotAllowed,
            "the method is not part of the S3 API",
        )
    })?;
    let mut headers = Vec::with_capacity(parts.headers.len());
    for (name, value) in &parts.headers {
        let value = value.to_str().map_err(|_| {
            Error::new(
                ErrorCode::InvalidArgument,
                "a header value is not visible ASCII",
            )
        })?;
        headers.push((name.as_str(), value));
    }
    let host = parts
        .headers
        .get("host")
        .and_then(|value| value.to_str().ok())
        .or_else(|| parts.uri.authority().map(|authority| authority.as_str()))
        .ok_or_else(|| Error::new(ErrorCode::InvalidRequest, "the Host header is required"))?;
    let raw_path = parts.uri.path();
    let raw_query = parts.uri.query().unwrap_or("");
    let addressed = resolve(host, raw_path, state.domains())?;
    if trail.logged {
        trail.bucket = addressed
            .bucket
            .as_ref()
            .map(|bucket| bucket.as_str().to_owned());
        trail.key = addressed.key.as_ref().map(|key| key.as_str().to_owned());
    }
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
    trail.operation = spec.name;
    let signed = SignedRequest {
        method: parts.method.as_str(),
        raw_path,
        raw_query,
        headers: &headers,
    };
    let (verified, principal) = authenticate(&state, &signed).await?;
    if trail.logged {
        trail.principal = Some(if verified.is_anonymous() {
            "anonymous".to_owned()
        } else {
            verified.access_key().to_owned()
        });
    }
    if !is_implemented(spec.operation) {
        return Err(Error::new(
            ErrorCode::NotImplemented,
            format!("{} is not implemented", spec.name),
        ));
    }
    let replica = parts
        .headers
        .contains_key(crate::routes::objects::REPLICA_VERSION);
    let secure = parts.extensions.get::<crate::OverTls>().is_some();
    let context = context(&state, &parts, &query, spec.operation, &verified, secure);
    authorize_operation(
        &state,
        spec.operation,
        &principal,
        &addressed,
        &parts.headers,
        query.iter().any(|(name, _)| name == "versionId") || replica,
        &context,
    )
    .await
    .inspect_err(|error| {
        if error.code == ErrorCode::AccessDenied {
            // A request with no credentials that nothing grants is refused for want of them, as it always was.
            if matches!(principal, Principal::Anonymous) {
                state.metrics().auth_refused(AuthRefusal::Anonymous);
            } else {
                state.metrics().denied(spec.name);
            }
        }
    })?;
    writable(&state, method, replica).await?;
    let call = Call {
        state: &state,
        addressed: &addressed,
        query: &query,
        verified: &verified,
        principal: &principal,
        headers: &parts.headers,
        secure,
        context: &context,
    };
    let answered = routes::route(spec.operation, &call, body).await;
    // A key's absence is told only to a caller who may list the bucket (S3 ref 04 M7); anyone else hears it is denied.
    match (answered, addressed.bucket.as_ref()) {
        (Err(error), Some(bucket))
            if matches!(error.code, ErrorCode::NoSuchKey | ErrorCode::NoSuchVersion)
                && !may_list(&state, &principal, bucket, &context).await? =>
        {
            Err(Error::new(ErrorCode::AccessDenied, "Access Denied"))
        }
        (answered, _) => answered,
    }
}

/// What a bucket policy's conditions read of this request: its transport, peer, time, how it was signed, the
/// headers a write declares and, on a listing, its prefix, delimiter and page size.
fn context(
    state: &ApiState,
    parts: &axum::http::request::Parts,
    query: &[(String, Option<String>)],
    operation: tessari_s3_core::dispatch::Operation,
    verified: &tessari_s3_core::auth::Verified,
    secure: bool,
) -> Context {
    use tessari_s3_core::dispatch::Operation as O;
    let header = |name: &str| {
        parts
            .headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };
    let value = |name: &str| {
        query
            .iter()
            .find(|(n, _)| n == name)
            .and_then(|(_, value)| value.clone())
    };
    let presigned = query.iter().any(|(name, _)| name == "X-Amz-Signature");
    let listing = matches!(
        operation,
        O::ListObjects | O::ListObjectsV2 | O::ListObjectVersions
    );
    Context {
        secure_transport: secure,
        source_ip: parts
            .extensions
            .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
            .map(|axum::extract::ConnectInfo(address)| address.ip()),
        now_secs: state.now(),
        auth_type: match (verified.is_anonymous(), presigned) {
            (true, _) => None,
            (false, true) => Some("REST-QUERY-STRING"),
            (false, false) => Some("REST-HEADER"),
        },
        signature_version: (!verified.is_anonymous()).then_some("AWS4-HMAC-SHA256"),
        content_sha256: header("x-amz-content-sha256"),
        server_side_encryption: header("x-amz-server-side-encryption"),
        prefix: listing.then(|| value("prefix").unwrap_or_default()),
        delimiter: if listing { value("delimiter") } else { None },
        max_keys: if listing {
            Some(
                value("max-keys")
                    .and_then(|text| text.parse().ok())
                    .unwrap_or(1000),
            )
        } else {
            None
        },
    }
}
