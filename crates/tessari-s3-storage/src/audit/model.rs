//! What one served request leaves in the access log. It names who asked and what was answered, and never carries a
//! credential: no Authorization header, no query string (a presigned URL's signature is there), no header value.

/// One request, as the access log records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessRecord {
    /// The request id the response carried; the record's identity.
    pub request_id: String,
    /// When the request arrived, in seconds since the Unix epoch by the node's clock.
    pub at_secs: i64,
    /// The HTTP method.
    pub method: String,
    /// The operation it was dispatched to, or `Unknown` when it was refused before dispatch.
    pub operation: &'static str,
    /// The bucket it addressed, if any.
    pub bucket: Option<String>,
    /// The object key it addressed, if any.
    pub key: Option<String>,
    /// The access key that signed it, or `anonymous`.
    pub principal: String,
    /// The HTTP status answered.
    pub status: u16,
    /// The S3 error code, or `None` for a success.
    pub code: &'static str,
    /// The larger of the request body and the response body, in bytes.
    pub bytes: u64,
    /// Milliseconds from arrival to the response.
    pub millis: u64,
    /// The client's address, when the connection had one.
    pub client: Option<String>,
}
