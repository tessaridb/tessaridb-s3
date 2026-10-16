//! What a request brings to a policy's conditions: who asks, and the facts the server populates for every request.
//! A key the server does not populate is refused when a policy is written, never evaluated as false.

use std::net::IpAddr;

use super::super::model::SpaceName;

/// Who asks, as a policy sees them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Requester<'a> {
    /// A request that carried no credentials.
    Anonymous,
    /// A signed-in user of a space.
    User {
        /// Their space.
        space: &'a SpaceName,
        /// Their name.
        name: &'a str,
    },
}

/// The request's facts a condition can test.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Context {
    /// `aws:SecureTransport`: the request arrived over TLS.
    pub secure_transport: bool,
    /// `aws:SourceIp`: the peer address of the connection — no forwarded header is believed.
    pub source_ip: Option<IpAddr>,
    /// `aws:CurrentTime` / `aws:EpochTime`, seconds since the epoch.
    pub now_secs: i64,
    /// `s3:authType`: `REST-HEADER`, `REST-QUERY-STRING` or `POST`; absent for an anonymous request.
    pub auth_type: Option<&'static str>,
    /// `s3:signatureversion`: `AWS4-HMAC-SHA256` when signed.
    pub signature_version: Option<&'static str>,
    /// `s3:x-amz-content-sha256`, as the request declared it.
    pub content_sha256: Option<String>,
    /// `s3:x-amz-server-side-encryption`, on a write that names one.
    pub server_side_encryption: Option<String>,
    /// `s3:prefix`, on a listing.
    pub prefix: Option<String>,
    /// `s3:delimiter`, on a listing that names one.
    pub delimiter: Option<String>,
    /// `s3:max-keys`, on a listing.
    pub max_keys: Option<i64>,
}

/// The condition keys this server populates; any other key in a policy is refused when the policy is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Key {
    SecureTransport,
    SourceIp,
    CurrentTime,
    EpochTime,
    Username,
    PrincipalType,
    AuthType,
    SignatureVersion,
    ContentSha256,
    ServerSideEncryption,
    Prefix,
    Delimiter,
    MaxKeys,
}

/// How a key's value compares, which decides the operators that may test it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Text,
    Number,
    Instant,
    Flag,
    Address,
}

/// A key's value in one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Observed<'a> {
    Text(std::borrow::Cow<'a, str>),
    Number(i64),
    Flag(bool),
    Address(IpAddr),
}

impl Key {
    /// The key a policy names, ignoring case as IAM does; `None` for one this server does not populate.
    pub(super) fn parse(name: &str) -> Option<Self> {
        Some(match name.to_ascii_lowercase().as_str() {
            "aws:securetransport" => Self::SecureTransport,
            "aws:sourceip" => Self::SourceIp,
            "aws:currenttime" => Self::CurrentTime,
            "aws:epochtime" => Self::EpochTime,
            "aws:username" => Self::Username,
            "aws:principaltype" => Self::PrincipalType,
            "s3:authtype" => Self::AuthType,
            "s3:signatureversion" => Self::SignatureVersion,
            "s3:x-amz-content-sha256" => Self::ContentSha256,
            "s3:x-amz-server-side-encryption" => Self::ServerSideEncryption,
            "s3:prefix" => Self::Prefix,
            "s3:delimiter" => Self::Delimiter,
            "s3:max-keys" => Self::MaxKeys,
            _ => return None,
        })
    }

    pub(super) const fn kind(self) -> Kind {
        match self {
            Self::SecureTransport => Kind::Flag,
            Self::SourceIp => Kind::Address,
            Self::CurrentTime | Self::EpochTime => Kind::Instant,
            Self::MaxKeys => Kind::Number,
            _ => Kind::Text,
        }
    }

    /// The key's value in `context` for `requester`; `None` when the request has none.
    pub(super) fn observe<'a>(
        self,
        requester: Requester<'a>,
        context: &'a Context,
    ) -> Option<Observed<'a>> {
        let text = |value: &'a str| Some(Observed::Text(std::borrow::Cow::Borrowed(value)));
        match self {
            Self::SecureTransport => Some(Observed::Flag(context.secure_transport)),
            Self::SourceIp => context.source_ip.map(Observed::Address),
            Self::CurrentTime | Self::EpochTime => Some(Observed::Number(context.now_secs)),
            Self::Username => match requester {
                Requester::User { name, .. } => text(name),
                Requester::Anonymous => None,
            },
            Self::PrincipalType => text(match requester {
                Requester::User { .. } => "User",
                Requester::Anonymous => "Anonymous",
            }),
            Self::AuthType => context.auth_type.and_then(text),
            Self::SignatureVersion => context.signature_version.and_then(text),
            Self::ContentSha256 => context.content_sha256.as_deref().and_then(text),
            Self::ServerSideEncryption => context.server_side_encryption.as_deref().and_then(text),
            Self::Prefix => context.prefix.as_deref().and_then(text),
            Self::Delimiter => context.delimiter.as_deref().and_then(text),
            Self::MaxKeys => context.max_keys.map(Observed::Number),
        }
    }
}
