//! GetBucketNotificationConfiguration and PutBucketNotificationConfiguration (S3 ref 11 M5). A configuration is
//! stored whole or not at all, names only destinations the operator declared, and an empty one turns notifications
//! off.

use std::fmt::Write as _;

use axum::body::Body;
use axum::http::{Response, StatusCode};
use tessari_s3_core::notification::{arn, validate};
use tessari_s3_types::ErrorCode;

use super::buckets::{existing, no_such_bucket};
use crate::pipeline::body::{CONTROL_BODY_LIMIT, read_verified};
use crate::pipeline::call::Call;
use crate::pipeline::response::{empty_response, xml_response};
use crate::xml::{S3_NAMESPACE, escape};
use crate::{Error, Result};

mod document;

/// `GET /{bucket}?notification` — an empty configuration while none is set, as S3 answers.
pub(crate) async fn get(call: &Call<'_>) -> Result<Response<Body>> {
    let bucket = existing(call).await?;
    let mut out = format!("<NotificationConfiguration xmlns=\"{S3_NAMESPACE}\">");
    for rule in bucket.notification.iter().flatten() {
        let _ = write!(
            out,
            "<QueueConfiguration><Id>{}</Id><Queue>{}</Queue>",
            escape(&rule.id),
            escape(&arn(&rule.target))
        );
        for event in &rule.events {
            let _ = write!(out, "<Event>{}</Event>", escape(event));
        }
        if !rule.prefix.is_empty() || !rule.suffix.is_empty() {
            out.push_str("<Filter><S3Key>");
            for (name, value) in [("prefix", &rule.prefix), ("suffix", &rule.suffix)] {
                if !value.is_empty() {
                    let _ = write!(
                        out,
                        "<FilterRule><Name>{name}</Name><Value>{}</Value></FilterRule>",
                        escape(value)
                    );
                }
            }
            out.push_str("</S3Key></Filter>");
        }
        out.push_str("</QueueConfiguration>");
    }
    out.push_str("</NotificationConfiguration>");
    Ok(xml_response(StatusCode::OK, out))
}

/// `PUT /{bucket}?notification` — the whole configuration replaces the one before it.
pub(crate) async fn put(call: &Call<'_>, body: Body) -> Result<Response<Body>> {
    let name = call.bucket()?;
    let document = read_verified(body, call.verified, CONTROL_BODY_LIMIT).await?;
    let rules: Vec<_> = document::parse(&document)?
        .into_iter()
        .map(|(id, mut rule)| {
            // A rule sent without an Id is given one, as S3 does.
            rule.id = id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            rule
        })
        .collect();
    validate(&rules, |target| call.state.declares_target(target))
        .map_err(|refused| Error::new(ErrorCode::InvalidArgument, refused.to_string()))?;
    if !call
        .state
        .storage()
        .buckets()
        .set_notification(name, &rules)
        .await?
    {
        return Err(no_such_bucket());
    }
    Ok(empty_response(StatusCode::OK))
}
