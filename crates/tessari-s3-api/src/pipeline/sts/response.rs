//! The STS query protocol's answers: the result document carrying the credentials, and the error document, both in
//! the STS namespace. The credentials are shown here once, so the answer is never stored by a cache.

use axum::body::Body;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderValue, Response, StatusCode};

use super::StsError;
use crate::xml::escape;

const NAMESPACE: &str = "https://sts.amazonaws.com/doc/2011-06-15/";

/// What the result document says.
pub(super) struct Assumed<'a> {
    pub(super) access_key_id: &'a str,
    pub(super) secret: &'a str,
    pub(super) session_token: &'a str,
    /// `Expiration`, ISO 8601.
    pub(super) expiration: &'a str,
    pub(super) subject: &'a str,
    /// `<space>`, `<user>` and the session name, for the assumed-role ARN and id.
    pub(super) space: &'a str,
    pub(super) user: &'a str,
    pub(super) session_name: &'a str,
    pub(super) provider: &'a str,
    pub(super) audience: &'a str,
}

fn xml(status: StatusCode, document: String) -> Response<Body> {
    let mut response = Response::new(Body::from(document));
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/xml"));
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// The `AssumeRoleWithWebIdentityResponse` for `assumed`.
pub(super) fn assumed(assumed: &Assumed<'_>, request_id: &str) -> Response<Body> {
    let Assumed {
        access_key_id,
        secret,
        session_token,
        expiration,
        subject,
        space,
        user,
        session_name,
        provider,
        audience,
    } = assumed;
    let role_arn = format!("arn:aws:sts::{space}:assumed-role/{user}/{session_name}");
    let document = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
<AssumeRoleWithWebIdentityResponse xmlns=\"{NAMESPACE}\"><AssumeRoleWithWebIdentityResult>\
<Credentials><AccessKeyId>{}</AccessKeyId><SecretAccessKey>{}</SecretAccessKey>\
<SessionToken>{}</SessionToken><Expiration>{}</Expiration></Credentials>\
<SubjectFromWebIdentityToken>{}</SubjectFromWebIdentityToken>\
<AssumedRoleUser><Arn>{}</Arn><AssumedRoleId>{}:{}</AssumedRoleId></AssumedRoleUser>\
<Provider>{}</Provider><Audience>{}</Audience>\
</AssumeRoleWithWebIdentityResult><ResponseMetadata><RequestId>{}</RequestId></ResponseMetadata>\
</AssumeRoleWithWebIdentityResponse>",
        escape(access_key_id),
        escape(secret),
        escape(session_token),
        escape(expiration),
        escape(subject),
        escape(&role_arn),
        escape(user),
        escape(session_name),
        escape(provider),
        escape(audience),
        escape(request_id),
    );
    xml(StatusCode::OK, document)
}

/// The `ErrorResponse` for `error`.
pub(super) fn refused(error: &StsError, request_id: &str) -> Response<Body> {
    let document = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
<ErrorResponse xmlns=\"{NAMESPACE}\"><Error><Type>Sender</Type><Code>{}</Code><Message>{}</Message></Error>\
<RequestId>{}</RequestId></ErrorResponse>",
        escape(error.code),
        escape(&error.message),
        escape(request_id),
    );
    xml(error.status, document)
}
