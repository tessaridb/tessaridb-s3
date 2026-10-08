//! Bucket operations: CreateBucket, HeadBucket, DeleteBucket, ListBuckets, GetBucketLocation.

use axum::body::Body;
use axum::http::{HeaderValue, Response, StatusCode, header};
use tessari_s3_core::authz::{SpaceName, Visible};
use tessari_s3_storage::buckets::{Bucket, Created, Deleted};
use tessari_s3_types::ErrorCode;

use crate::pipeline::body::{CONTROL_BODY_LIMIT, read_verified};
use crate::pipeline::call::Call;
use crate::pipeline::response::{empty_response, xml_response};
use crate::xml::{S3_NAMESPACE, element_text, escape};
use crate::{Error, Result};

/// Most buckets one ListBuckets page may carry.
const MAX_BUCKETS_PER_PAGE: usize = 10_000;

/// `PUT /{bucket}` — an optional `CreateBucketConfiguration` may only name this server's region.
pub(crate) async fn create(call: &Call<'_>, body: Body) -> Result<Response<Body>> {
    let name = call.bucket()?;
    let document = read_verified(body, call.verified, CONTROL_BODY_LIMIT).await?;
    let region = call.state.region();
    let constraint = element_text(&document, "CreateBucketConfiguration", "LocationConstraint")?;
    if constraint.is_some_and(|constraint| !constraint.is_empty() && constraint != region) {
        return Err(Error::new(
            ErrorCode::IllegalLocationConstraintException,
            format!("this server creates buckets in {region} only"),
        ));
    }
    // Until signed-in users exist the caller is the root credential: its buckets belong to the default space.
    let space = SpaceName::default_space();
    match call
        .state
        .storage()
        .buckets()
        .create(name, region, &space, None)
        .await?
    {
        Created::Created(_) => {
            let mut response = empty_response(StatusCode::OK);
            if let Ok(location) = HeaderValue::from_str(&format!("/{}", name.as_str())) {
                response.headers_mut().insert(header::LOCATION, location);
            }
            Ok(response)
        }
        Created::AlreadyOwned => Err(Error::new(
            ErrorCode::BucketAlreadyOwnedByYou,
            "the bucket you tried to create already exists, and you own it",
        )),
    }
}

/// `HEAD /{bucket}` — whether it exists, and its region.
pub(crate) async fn head(call: &Call<'_>) -> Result<Response<Body>> {
    let bucket = existing(call).await?;
    let mut response = empty_response(StatusCode::OK);
    if let Ok(region) = HeaderValue::from_str(&bucket.region) {
        response.headers_mut().insert("x-amz-bucket-region", region);
    }
    Ok(response)
}

/// `DELETE /{bucket}` — only when it holds no objects.
pub(crate) async fn delete(call: &Call<'_>) -> Result<Response<Body>> {
    match call
        .state
        .storage()
        .buckets()
        .delete(call.bucket()?)
        .await?
    {
        Deleted::Deleted => Ok(empty_response(StatusCode::NO_CONTENT)),
        Deleted::NoSuchBucket => Err(no_such_bucket()),
        Deleted::NotEmpty => Err(Error::new(
            ErrorCode::BucketNotEmpty,
            "the bucket you tried to delete is not empty",
        )),
    }
}

/// `GET /?location` — the bucket's region; AWS answers an empty element for us-east-1.
pub(crate) async fn location(call: &Call<'_>) -> Result<Response<Body>> {
    let bucket = existing(call).await?;
    let region = if bucket.region == "us-east-1" {
        ""
    } else {
        bucket.region.as_str()
    };
    Ok(xml_response(
        StatusCode::OK,
        format!(
            "<LocationConstraint xmlns=\"{S3_NAMESPACE}\">{}</LocationConstraint>",
            escape(region)
        ),
    ))
}

/// `GET /` — every bucket in byte order of name, filtered by `prefix` and `bucket-region`, paged by `max-buckets`
/// with an opaque continuation token.
pub(crate) async fn list(call: &Call<'_>) -> Result<Response<Body>> {
    let prefix = call.query_value("prefix").unwrap_or("");
    let region = call.query_value("bucket-region");
    let max = match call.query_value("max-buckets") {
        None => MAX_BUCKETS_PER_PAGE,
        Some(text) => text
            .parse::<usize>()
            .ok()
            .filter(|max| (1..=MAX_BUCKETS_PER_PAGE).contains(max))
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::InvalidArgument,
                    "max-buckets must be between 1 and 10000",
                )
            })?,
    };
    let after = match call.query_value("continuation-token") {
        None => None,
        Some(token) => Some(decode_token(token)?),
    };
    let buckets = call.state.storage().buckets().list(&Visible::All).await?;
    let mut page: Vec<&Bucket> = buckets
        .iter()
        .filter(|b| b.name.as_str().starts_with(prefix))
        .filter(|b| region.is_none_or(|region| b.region == region))
        .filter(|b| {
            after
                .as_deref()
                .is_none_or(|after| b.name.as_str().as_bytes() > after.as_bytes())
        })
        .take(max.saturating_add(1))
        .collect();
    let truncated = page.len() > max;
    page.truncate(max);
    let mut document = format!(
        "<ListAllMyBucketsResult xmlns=\"{S3_NAMESPACE}\"><Owner><ID>{owner}</ID><DisplayName>{owner}</DisplayName>\
         </Owner><Buckets>",
        owner = escape(call.verified.access_key())
    );
    for bucket in &page {
        document.push_str(&format!(
            "<Bucket><Name>{}</Name><CreationDate>{}</CreationDate><BucketRegion>{}</BucketRegion></Bucket>",
            escape(bucket.name.as_str()),
            bucket.created.iso8601_millis(),
            escape(&bucket.region)
        ));
    }
    document.push_str("</Buckets>");
    if let Some(last) = page.last().filter(|_| truncated) {
        document.push_str(&format!(
            "<ContinuationToken>{}</ContinuationToken>",
            encode_token(last.name.as_str())
        ));
    }
    document.push_str(&format!(
        "<Prefix>{}</Prefix></ListAllMyBucketsResult>",
        escape(prefix)
    ));
    Ok(xml_response(StatusCode::OK, document))
}

/// The addressed bucket, or `NoSuchBucket`.
async fn existing(call: &Call<'_>) -> Result<Bucket> {
    call.state
        .storage()
        .buckets()
        .get(call.bucket()?)
        .await?
        .ok_or_else(no_such_bucket)
}

fn no_such_bucket() -> Error {
    Error::new(
        ErrorCode::NoSuchBucket,
        "the specified bucket does not exist",
    )
}

/// A continuation token: the last bucket name, hex-encoded so clients treat it as opaque.
fn encode_token(name: &str) -> String {
    name.bytes().map(|b| format!("{b:02x}")).collect()
}

fn decode_token(token: &str) -> Result<String> {
    let invalid = || {
        Error::new(
            ErrorCode::InvalidArgument,
            "the continuation token is not one this server issued",
        )
    };
    let bytes = token.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return Err(invalid());
    }
    let (pairs, _) = bytes.as_chunks::<2>();
    let mut decoded = Vec::with_capacity(pairs.len());
    for pair in pairs {
        let text = std::str::from_utf8(pair).map_err(|_| invalid())?;
        decoded.push(u8::from_str_radix(text, 16).map_err(|_| invalid())?);
    }
    String::from_utf8(decoded).map_err(|_| invalid())
}

#[cfg(test)]
mod tests {
    use super::{decode_token, encode_token};

    #[test]
    fn a_continuation_token_round_trips_and_a_forged_one_is_refused() {
        assert_eq!(
            decode_token(&encode_token("my-bucket.2")).ok().as_deref(),
            Some("my-bucket.2")
        );
        for forged in ["abc", "zz", "ff"] {
            assert!(decode_token(forged).is_err(), "{forged}");
        }
    }
}
