//! ACLs and Object Ownership with ACLs disabled (S3 ref 04, ownership BucketOwnerEnforced): `GET ?acl` on a bucket or
//! an object answers its owner's FULL_CONTROL, `PUT ?acl` accepts only the ACLs that grant the owner alone (`private`,
//! `bucket-owner-full-control`, or none) and changes nothing, and the ownership controls are BucketOwnerEnforced.
//! Policies decide every request; a grant written here would be decided by nothing, so it is refused.

use axum::body::Body;
use axum::http::{Response, StatusCode};
use tessari_s3_types::{ErrorCode, ObjectKey};

use super::buckets::existing;
use super::objects::addressed;
use crate::pipeline::body::{CONTROL_BODY_LIMIT, read_verified};
use crate::pipeline::call::Call;
use crate::pipeline::response::{empty_response, xml_response};
use crate::xml::{S3_NAMESPACE, element_text, escape};
use crate::{Error, Result};

const ENFORCED: &str = "BucketOwnerEnforced";

/// The one ACL there is: the owner — the space, as every owner this server answers names — holding FULL_CONTROL.
fn owner_full_control(call: &Call<'_>) -> Response<Body> {
    let owner = escape(&call.owner());
    xml_response(
        StatusCode::OK,
        format!(
            "<AccessControlPolicy xmlns=\"{S3_NAMESPACE}\"><Owner><ID>{owner}</ID><DisplayName>{owner}</DisplayName>\
             </Owner><AccessControlList><Grant><Grantee xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" \
             xsi:type=\"CanonicalUser\"><ID>{owner}</ID><DisplayName>{owner}</DisplayName></Grantee>\
             <Permission>FULL_CONTROL</Permission></Grant></AccessControlList></AccessControlPolicy>"
        ),
    )
}

/// Refuses an ACL write that would grant anyone but the owner: a grant header, a canned ACL other than the owner's
/// own, or a document in the body.
fn owner_only(call: &Call<'_>, document: &[u8]) -> Result<()> {
    let refused = || {
        Error::new(
            ErrorCode::AccessControlListNotSupported,
            "The bucket does not allow ACLs",
        )
    };
    if call
        .headers
        .keys()
        .any(|name| name.as_str().starts_with("x-amz-grant-"))
        || !document.is_empty()
    {
        return Err(refused());
    }
    match call.headers.get("x-amz-acl").map(|v| v.to_str()) {
        None | Some(Ok("private" | "bucket-owner-full-control")) => Ok(()),
        Some(_) => Err(refused()),
    }
}

/// `GET /{bucket}?acl`.
pub(crate) async fn get_bucket(call: &Call<'_>) -> Result<Response<Body>> {
    existing(call).await?;
    Ok(owner_full_control(call))
}

/// `PUT /{bucket}?acl`.
pub(crate) async fn put_bucket(call: &Call<'_>, body: Body) -> Result<Response<Body>> {
    owner_only(
        call,
        &read_verified(body, call.verified, CONTROL_BODY_LIMIT).await?,
    )?;
    existing(call).await?;
    Ok(empty_response(StatusCode::OK))
}

/// `GET /{bucket}/{key}?acl`, of the version `versionId` names or the current one.
pub(crate) async fn get_object(call: &Call<'_>, key: &ObjectKey) -> Result<Response<Body>> {
    addressed(call, key).await?;
    Ok(owner_full_control(call))
}

/// `PUT /{bucket}/{key}?acl`.
pub(crate) async fn put_object(
    call: &Call<'_>,
    key: &ObjectKey,
    body: Body,
) -> Result<Response<Body>> {
    owner_only(
        call,
        &read_verified(body, call.verified, CONTROL_BODY_LIMIT).await?,
    )?;
    addressed(call, key).await?;
    Ok(empty_response(StatusCode::OK))
}

/// `GET /{bucket}?ownershipControls`.
pub(crate) async fn get_ownership(call: &Call<'_>) -> Result<Response<Body>> {
    existing(call).await?;
    Ok(xml_response(
        StatusCode::OK,
        format!(
            "<OwnershipControls xmlns=\"{S3_NAMESPACE}\"><Rule><ObjectOwnership>{ENFORCED}</ObjectOwnership></Rule>\
             </OwnershipControls>"
        ),
    ))
}

/// `PUT /{bucket}?ownershipControls`: BucketOwnerEnforced is what every bucket has; another setting would turn ACLs
/// on, which this server does not do.
pub(crate) async fn put_ownership(call: &Call<'_>, body: Body) -> Result<Response<Body>> {
    let document = read_verified(body, call.verified, CONTROL_BODY_LIMIT).await?;
    match element_text(&document, "OwnershipControls", "ObjectOwnership")?.as_deref() {
        Some(ENFORCED) => {}
        Some("ObjectWriter" | "BucketOwnerPreferred") => {
            return Err(Error::new(
                ErrorCode::NotImplemented,
                "only BucketOwnerEnforced is implemented: ACLs stay disabled",
            ));
        }
        _ => {
            return Err(Error::new(
                ErrorCode::MalformedXml,
                "the ownership controls name one ObjectOwnership",
            ));
        }
    }
    existing(call).await?;
    Ok(empty_response(StatusCode::OK))
}
