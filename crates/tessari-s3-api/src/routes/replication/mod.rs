//! GetBucketReplication, PutBucketReplication and DeleteBucketReplication. A configuration is stored whole or not at
//! all; it needs versioning Enabled on the bucket and on every destination, as S3 does (S3 ref 11 M4).

use std::fmt::Write as _;

use axum::body::Body;
use axum::http::{Response, StatusCode};
use tessari_s3_core::replication::validate;
use tessari_s3_core::versioning::Versioning;
use tessari_s3_types::{BucketName, ErrorCode};

use super::buckets::{existing, no_such_bucket};
use crate::pipeline::body::{CONTROL_BODY_LIMIT, read_verified};
use crate::pipeline::call::Call;
use crate::pipeline::response::{empty_response, xml_response};
use crate::xml::{S3_NAMESPACE, escape};
use crate::{Error, Result};

mod document;

fn invalid(message: impl Into<String>) -> Error {
    Error::new(ErrorCode::InvalidRequest, message)
}

/// `GET /{bucket}?replication`.
pub(crate) async fn get(call: &Call<'_>) -> Result<Response<Body>> {
    let Some(configuration) = existing(call).await?.replication else {
        return Err(Error::new(
            ErrorCode::ReplicationConfigurationNotFoundError,
            "The replication configuration was not found",
        ));
    };
    let mut out = format!(
        "<ReplicationConfiguration xmlns=\"{S3_NAMESPACE}\"><Role>{}</Role>",
        escape(&configuration.role)
    );
    for rule in &configuration.rules {
        let _ = write!(
            out,
            "<Rule><ID>{}</ID><Priority>{}</Priority><Status>{}</Status><Filter><Prefix>{}</Prefix></Filter>\
             <DeleteMarkerReplication><Status>{}</Status></DeleteMarkerReplication>\
             <Destination><Bucket>arn:aws:s3:::{}</Bucket></Destination></Rule>",
            escape(&rule.id),
            rule.priority,
            if rule.enabled { "Enabled" } else { "Disabled" },
            escape(&rule.prefix),
            if rule.delete_markers {
                "Enabled"
            } else {
                "Disabled"
            },
            escape(&rule.destination),
        );
    }
    out.push_str("</ReplicationConfiguration>");
    Ok(xml_response(StatusCode::OK, out))
}

/// `PUT /{bucket}?replication` — the whole configuration replaces the one before it.
pub(crate) async fn put(call: &Call<'_>, body: Body) -> Result<Response<Body>> {
    let name = call.bucket()?;
    let document = read_verified(body, call.verified, CONTROL_BODY_LIMIT).await?;
    let configuration = document::parse(&document)?;
    validate(&configuration, name.as_str()).map_err(|refused| invalid(refused.to_string()))?;
    crate::routes::buckets::resident(call).await?;
    let source = existing(call).await?;
    if source.versioning != Some(Versioning::Enabled) {
        return Err(invalid(
            "Versioning must be 'Enabled' on the bucket to apply a replication configuration",
        ));
    }
    let buckets = call.state.storage().buckets();
    for rule in &configuration.rules {
        let target = BucketName::new(&rule.destination)
            .map_err(|_| invalid("The destination bucket name is not a bucket name"))?;
        match buckets.get(&target).await? {
            None => return Err(invalid("The destination bucket must exist")),
            Some(bucket) if bucket.versioning != Some(Versioning::Enabled) => {
                return Err(invalid(
                    "The destination bucket must have versioning enabled",
                ));
            }
            Some(_) => {}
        }
    }
    if !buckets.set_replication(name, Some(&configuration)).await? {
        return Err(no_such_bucket());
    }
    Ok(empty_response(StatusCode::OK))
}

/// `DELETE /{bucket}?replication`.
pub(crate) async fn delete(call: &Call<'_>) -> Result<Response<Body>> {
    if !call
        .state
        .storage()
        .buckets()
        .set_replication(call.bucket()?, None)
        .await?
    {
        return Err(no_such_bucket());
    }
    Ok(empty_response(StatusCode::NO_CONTENT))
}
