//! The console's page and its assets. They are served without a session because they carry no data — everything the
//! page shows it fetches from `/api/v1` with the session cookie — and each answer carries a policy that lets the page
//! run only its own script and style, talk only to this origin, and never be framed. Like every public route they are
//! limited per client address.
//!
//! The bytes are the committed output of `panel/` (`npm run build`), embedded so that building the server never needs
//! Node; `npm run verify` fails when the committed output no longer matches its source.

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, State};
use axum::http::header::{
    CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE, REFERRER_POLICY, X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{HeaderValue, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::get;

use super::ConsoleState;
use super::error::ConsoleError;

/// What the page may load and do: its own script, style and icon; requests to this origin only; no forms submitted
/// anywhere, no `<base>` rewriting, no framing.
const POLICY: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; connect-src 'self'; \
                      form-action 'none'; frame-ancestors 'none'; base-uri 'none'";

/// One asset: the path it is served at, its type, its bytes.
type Asset = (&'static str, &'static str, &'static str);

const ASSETS: [Asset; 4] = [
    (
        "/",
        "text/html; charset=utf-8",
        include_str!("../../assets/index.html"),
    ),
    (
        "/console.js",
        "text/javascript; charset=utf-8",
        include_str!("../../assets/console.js"),
    ),
    (
        "/console.css",
        "text/css; charset=utf-8",
        include_str!("../../assets/console.css"),
    ),
    (
        "/favicon.svg",
        "image/svg+xml",
        include_str!("../../assets/favicon.svg"),
    ),
];

/// The page's routes, one per asset.
pub(super) fn routes() -> axum::Router<ConsoleState> {
    ASSETS
        .iter()
        .fold(axum::Router::new(), |router, (path, _, _)| {
            router.route(path, get(asset))
        })
}

async fn asset(
    State(state): State<ConsoleState>,
    ConnectInfo(client): ConnectInfo<SocketAddr>,
    uri: Uri,
) -> Result<Response, ConsoleError> {
    // Keyed apart from signed-in key ids, which never start with this prefix.
    let key = format!("page:{}", client.ip());
    if !state
        .requests()
        .admit(&key, state.now())
        .map_err(|()| ConsoleError::internal())?
    {
        return Err(ConsoleError::rate_limited());
    }
    let (_, kind, body) = ASSETS
        .iter()
        .find(|(path, _, _)| *path == uri.path())
        .ok_or(ConsoleError::not_found())?;
    let mut response = (*body).into_response();
    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(kind));
    headers.insert(CONTENT_SECURITY_POLICY, HeaderValue::from_static(POLICY));
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    Ok(response)
}
