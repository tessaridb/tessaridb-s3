//! A minimal S3 client for the benchmark: every request signed with the server's own SigV4 code, the payload
//! declared by its SHA-256, and a refusal reported by its S3 error code.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use reqwest::{Method, Response};
use tessari_s3_constants::BENCH_REQUEST_TIMEOUT_SECS;
use tessari_s3_core::auth::{SignedRequest, amz_date, sign_header, uri_encode};
use tessari_s3_types::SecretKey;

use crate::error::{Error, Result};

/// SHA-256 of an empty body.
pub const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// Where to send requests and who signs them.
#[derive(Debug)]
pub struct S3Client {
    http: reqwest::Client,
    base: String,
    host: String,
    region: String,
    access_key: String,
    secret: SecretKey,
}

/// Why one request did not succeed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// The server answered with an error: its HTTP status and S3 code (the status for a bodiless answer).
    Refused {
        /// HTTP status.
        status: u16,
        /// S3 error code.
        code: String,
    },
    /// The request failed below HTTP: connect, reset, timeout.
    Transport,
}

impl Failure {
    /// The label a report counts it under.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Refused { status, code } => format!("{status} {code}"),
            Self::Transport => "transport".to_owned(),
        }
    }
}

impl S3Client {
    /// A client for `endpoint` (`host:port`, plain HTTP), signing for `region` as `access_key`.
    ///
    /// # Errors
    /// [`Error::Client`] when the HTTP stack cannot be set up.
    pub fn new(endpoint: &str, region: &str, access_key: &str, secret: SecretKey) -> Result<Self> {
        let tls =
            tessari_s3_infrastructure::tls::plain_client_config().map_err(|_| Error::Client)?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(BENCH_REQUEST_TIMEOUT_SECS))
            .tls_backend_preconfigured(tls)
            .build()
            .map_err(|_| Error::Client)?;
        Ok(Self {
            http,
            base: format!("http://{endpoint}"),
            host: endpoint.to_owned(),
            region: region.to_owned(),
            access_key: access_key.to_owned(),
            secret,
        })
    }

    /// Sends a signed request. `path` and `query` go on the wire exactly as given (already percent-encoded).
    ///
    /// # Errors
    /// [`Error::Sign`] if the request cannot be signed; [`Error::Transport`] below HTTP.
    pub async fn send(
        &self,
        method: Method,
        path: &str,
        query: &str,
        body: Option<Bytes>,
        sha256_hex: &str,
    ) -> Result<Response> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|elapsed| i64::try_from(elapsed.as_secs()).ok())
            .unwrap_or_default();
        let date = amz_date(now);
        let headers = [
            ("host", self.host.as_str()),
            ("x-amz-content-sha256", sha256_hex),
            ("x-amz-date", date.as_str()),
        ];
        let authorization = sign_header(
            &SignedRequest {
                method: method.as_str(),
                raw_path: path,
                raw_query: query,
                headers: &headers,
            },
            &self.access_key,
            &self.secret,
            &self.region,
        )?;
        let url = if query.is_empty() {
            format!("{}{path}", self.base)
        } else {
            format!("{}{path}?{query}", self.base)
        };
        let mut request = self
            .http
            .request(method, url)
            .header("x-amz-content-sha256", sha256_hex)
            .header("x-amz-date", date)
            .header("authorization", authorization);
        if let Some(body) = body {
            request = request.body(body);
        }
        Ok(request.send().await?)
    }

    /// Creates `bucket`, or accepts that this credential already owns it.
    ///
    /// # Errors
    /// [`Error::SetUp`] for any other refusal; transport and signing errors.
    pub async fn ensure_bucket(&self, bucket: &str) -> Result<()> {
        let response = self
            .send(Method::PUT, &format!("/{bucket}"), "", None, EMPTY_SHA256)
            .await?;
        match refusal(response).await {
            None => Ok(()),
            Some(Failure::Refused { code, .. }) if code == "BucketAlreadyOwnedByYou" => Ok(()),
            Some(Failure::Refused { status, code }) => Err(Error::SetUp { status, code }),
            Some(Failure::Transport) => Err(Error::SetUp {
                status: 0,
                code: "transport".to_owned(),
            }),
        }
    }
}

/// `None` for a 2xx answer; otherwise the refusal, its S3 code read from the error body.
pub async fn refusal(response: Response) -> Option<Failure> {
    let status = response.status();
    if status.is_success() {
        return None;
    }
    let text = response.text().await.unwrap_or_default();
    let code = text
        .split_once("<Code>")
        .and_then(|(_, rest)| rest.split_once("</Code>"))
        .map_or_else(|| status.as_u16().to_string(), |(code, _)| code.to_owned());
    Some(Failure::Refused {
        status: status.as_u16(),
        code,
    })
}

/// A query value percent-encoded as SigV4 canonicalises it, so the wire form and the signed form agree.
#[must_use]
pub fn encode_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    uri_encode(value.as_bytes(), false, &mut out);
    out
}
