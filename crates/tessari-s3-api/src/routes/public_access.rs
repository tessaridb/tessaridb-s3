//! Block Public Access (S3 ref 04 M5): `PUT`, `GET` and `DELETE /{bucket}?publicAccessBlock`. A bucket that never
//! stored one answers every switch on; a removed one blocks nothing and is answered as absent.

use axum::body::Body;
use axum::http::{Response, StatusCode};
use tessari_s3_core::authz::policy::PublicAccessBlock;
use tessari_s3_types::ErrorCode;

use super::buckets::{existing, no_such_bucket, resident};
use crate::pipeline::body::{CONTROL_BODY_LIMIT, read_verified};
use crate::pipeline::call::Call;
use crate::pipeline::response::{empty_response, xml_response};
use crate::xml::{S3_NAMESPACE, element_text};
use crate::{Error, Result};

const ROOT: &str = "PublicAccessBlockConfiguration";
const SWITCHES: [&str; 4] = [
    "BlockPublicAcls",
    "IgnorePublicAcls",
    "BlockPublicPolicy",
    "RestrictPublicBuckets",
];

/// `GET /{bucket}?publicAccessBlock`.
pub(crate) async fn get(call: &Call<'_>) -> Result<Response<Body>> {
    let block = existing(call)
        .await?
        .public_access
        .answered()
        .ok_or_else(|| {
            Error::new(
                ErrorCode::NoSuchPublicAccessBlockConfiguration,
                "The public access block configuration was not found",
            )
        })?;
    let values = [
        block.block_public_acls,
        block.ignore_public_acls,
        block.block_public_policy,
        block.restrict_public_buckets,
    ];
    let body: String = SWITCHES
        .iter()
        .zip(values)
        .map(|(name, on)| format!("<{name}>{on}</{name}>"))
        .collect();
    Ok(xml_response(
        StatusCode::OK,
        format!("<{ROOT} xmlns=\"{S3_NAMESPACE}\">{body}</{ROOT}>"),
    ))
}

/// `PUT /{bucket}?publicAccessBlock`: each switch `true` or `false`, an absent one off.
pub(crate) async fn put(call: &Call<'_>, body: Body) -> Result<Response<Body>> {
    let document = read_verified(body, call.verified, CONTROL_BODY_LIMIT).await?;
    let mut values = [false; 4];
    for (value, name) in values.iter_mut().zip(SWITCHES) {
        *value = match element_text(&document, ROOT, name)?.as_deref() {
            None | Some("false") => false,
            Some("true") => true,
            Some(_) => {
                return Err(Error::new(
                    ErrorCode::MalformedXml,
                    format!("{name} must be true or false"),
                ));
            }
        };
    }
    let [acls, ignore, policy, restrict] = values;
    resident(call).await?;
    set(
        call,
        Some(PublicAccessBlock {
            block_public_acls: acls,
            ignore_public_acls: ignore,
            block_public_policy: policy,
            restrict_public_buckets: restrict,
        }),
        StatusCode::OK,
    )
    .await
}

/// `DELETE /{bucket}?publicAccessBlock`: nothing is blocked afterwards.
pub(crate) async fn delete(call: &Call<'_>) -> Result<Response<Body>> {
    set(call, None, StatusCode::NO_CONTENT).await
}

async fn set(
    call: &Call<'_>,
    block: Option<PublicAccessBlock>,
    status: StatusCode,
) -> Result<Response<Body>> {
    if call
        .state
        .storage()
        .buckets()
        .set_public_access(call.bucket()?, block)
        .await?
    {
        Ok(empty_response(status))
    } else {
        Err(no_such_bucket())
    }
}
