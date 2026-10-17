//! An `AssumeRoleWithWebIdentity` request as the STS query protocol sends it: a form body, every parameter once, each
//! checked against the published limits before anything is believed. A parameter this server does not act on is
//! refused rather than ignored, because each of them would narrow or reshape the credentials it answers with.

use axum::http::StatusCode;
use tessari_s3_constants::access::SESSION_POLICY_MAX_CHARS;
use tessari_s3_core::auth::percent_decode;
use tessari_s3_core::authz::policy::SessionPolicy;

use super::StsError;

/// Largest `AssumeRoleWithWebIdentity` form body read: the identity token (at most 20 000 characters), a session
/// policy (2 048) and the rest, URL-encoded, fit with room to spare.
pub(super) const STS_MAX_BODY_BYTES: usize = 32 * 1024;
/// Shortest lifetime temporary credentials may be asked for, in seconds (the STS minimum).
const STS_MIN_DURATION_SECS: i64 = 900;
/// Longest lifetime temporary credentials may be asked for, in seconds (twelve hours, the STS maximum).
const STS_MAX_DURATION_SECS: i64 = 43_200;
/// Lifetime of temporary credentials when the request names none, in seconds.
const STS_DEFAULT_DURATION_SECS: i64 = 3_600;

const ACTION: &str = "AssumeRoleWithWebIdentity";
const VERSION: &str = "2011-06-15";

/// What the caller asks for.
#[derive(Debug)]
pub(super) struct AssumeRequest {
    pub(super) role_arn: String,
    pub(super) session_name: String,
    pub(super) token: String,
    pub(super) duration_secs: i64,
    /// The session policy, as written and already checked.
    pub(super) policy: Option<String>,
}

fn invalid(message: impl Into<String>) -> StsError {
    StsError::new(StatusCode::BAD_REQUEST, "ValidationError", message)
}

/// One form name or value: `+` is a space, then percent-decoded, then UTF-8.
fn decoded(raw: &str) -> Result<String, StsError> {
    let spaced = raw.replace('+', " ");
    percent_decode(&spaced)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .ok_or_else(|| invalid("the request body is not valid form-encoded UTF-8"))
}

/// `value`'s length in characters, checked against `min..=max`.
fn sized<'a>(name: &str, value: &'a str, min: usize, max: usize) -> Result<&'a str, StsError> {
    let length = value.chars().count();
    if (min..=max).contains(&length) {
        Ok(value)
    } else {
        Err(invalid(format!(
            "{name} must be {min} to {max} characters long"
        )))
    }
}

/// The request in `body`.
///
/// # Errors
/// `InvalidAction` for any other action, `MalformedPolicyDocument` for a session policy this server cannot evaluate,
/// `ValidationError` for a parameter missing, repeated, out of its limits or not known here.
pub(super) fn parse(body: &str) -> Result<AssumeRequest, StsError> {
    let mut fields: Vec<(String, String)> = Vec::new();
    for parameter in body.split('&').filter(|parameter| !parameter.is_empty()) {
        let (name, value) = parameter.split_once('=').unwrap_or((parameter, ""));
        let (name, value) = (decoded(name)?, decoded(value)?);
        if fields.iter().any(|(seen, _)| *seen == name) {
            return Err(invalid(format!("{name} is given more than once")));
        }
        fields.push((name, value));
    }
    let field = |name: &str| {
        fields
            .iter()
            .find(|(seen, _)| seen == name)
            .map(|(_, value)| value.as_str())
    };
    if field("Action") != Some(ACTION) {
        return Err(StsError::new(
            StatusCode::BAD_REQUEST,
            "InvalidAction",
            "only AssumeRoleWithWebIdentity is served here",
        ));
    }
    if field("Version").is_some_and(|version| version != VERSION) {
        return Err(invalid(format!("Version must be {VERSION}")));
    }
    let policy = match field("Policy") {
        None => None,
        Some(text) => {
            let text = sized("Policy", text, 1, SESSION_POLICY_MAX_CHARS)?;
            SessionPolicy::parse(text).map_err(|error| {
                StsError::new(
                    StatusCode::BAD_REQUEST,
                    "MalformedPolicyDocument",
                    error.to_string(),
                )
            })?;
            Some(text.to_owned())
        }
    };
    const KNOWN: [&str; 7] = [
        "Action",
        "Version",
        "Policy",
        "RoleArn",
        "RoleSessionName",
        "WebIdentityToken",
        "DurationSeconds",
    ];
    if let Some((unknown, _)) = fields
        .iter()
        .find(|(name, _)| !KNOWN.contains(&name.as_str()))
    {
        return Err(invalid(format!("{unknown} is not supported")));
    }
    let required = |name: &str| field(name).ok_or_else(|| invalid(format!("{name} is required")));
    let role_arn = sized("RoleArn", required("RoleArn")?, 20, 2048)?;
    let session_name = sized("RoleSessionName", required("RoleSessionName")?, 2, 64)?;
    if !session_name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "_+=,.@-".contains(c))
    {
        return Err(invalid(
            "RoleSessionName may hold only letters, digits and _+=,.@-",
        ));
    }
    let token = sized("WebIdentityToken", required("WebIdentityToken")?, 4, 20_000)?;
    let duration_secs = match field("DurationSeconds") {
        None => STS_DEFAULT_DURATION_SECS,
        Some(text) => text
            .parse::<i64>()
            .ok()
            .filter(|secs| (STS_MIN_DURATION_SECS..=STS_MAX_DURATION_SECS).contains(secs))
            .ok_or_else(|| {
                invalid(format!(
                    "DurationSeconds must be {STS_MIN_DURATION_SECS} to {STS_MAX_DURATION_SECS}"
                ))
            })?,
    };
    Ok(AssumeRequest {
        role_arn: role_arn.to_owned(),
        session_name: session_name.to_owned(),
        token: token.to_owned(),
        duration_secs,
        policy,
    })
}
