//! ListObjectsV2 and ListObjects (V1): one page of a bucket's keys in UTF-8 byte order, rolled up at a delimiter,
//! resumed by an authenticated continuation token (V2) or a marker (V1).

use axum::body::Body;
use axum::http::{Response, StatusCode};
use tessari_s3_constants::LIST_MAX_KEYS;
use tessari_s3_core::objects::listing::ListSpec;
use tessari_s3_core::objects::token::{self, TokenScope};
use tessari_s3_storage::objects::Listed;
use tessari_s3_types::ErrorCode;

use crate::pipeline::call::Call;
use crate::pipeline::response::xml_response;
use crate::{Error, Result};

mod document;

pub(crate) use document::listed_name;
use document::{Listing, Version};

fn invalid(message: &str) -> Error {
    Error::new(ErrorCode::InvalidArgument, message)
}

/// `max-keys`: a non-negative integer, at most 1,000 (a larger request gets 1,000, as in S3).
fn max_keys(call: &Call<'_>) -> Result<usize> {
    match call.query_value("max-keys") {
        None => Ok(LIST_MAX_KEYS),
        Some(text) => text
            .parse::<u64>()
            .map(|max| usize::try_from(max).map_or(LIST_MAX_KEYS, |max| max.min(LIST_MAX_KEYS)))
            .map_err(|_| invalid("max-keys must be a non-negative integer")),
    }
}

/// `encoding-type`: absent, or `url`.
pub(crate) fn url_encoded(call: &Call<'_>) -> Result<bool> {
    match call.query_value("encoding-type") {
        None => Ok(false),
        Some("url") => Ok(true),
        Some(_) => Err(invalid("encoding-type must be url")),
    }
}

/// `GET /{bucket}?list-type=2`.
pub(crate) async fn list_v2(call: &Call<'_>) -> Result<Response<Body>> {
    let token = call.query_value("continuation-token");
    let request = Request::read(call)?;
    let scope = request.scope(call)?;
    let resume = token
        .map(|token| {
            token::decode(call.state.list_tokens(), scope, token).map_err(|_| {
                invalid("the continuation token is not one this server issued for this listing")
            })
        })
        .transpose()?;
    let start_after = call.query_value("start-after");
    let listing = request
        .run(call, start_after.map(str::to_owned), resume)
        .await?;
    let next_token = listing
        .page
        .resume()
        .map(|resume| token::encode(call.state.list_tokens(), scope, &resume));
    let version = Version::Two {
        token,
        next_token: next_token.as_deref(),
        start_after,
        fetch_owner: call.query_value("fetch-owner") == Some("true"),
    };
    respond(call, &listing, &version)
}

/// `GET /{bucket}` — version 1, paged by `marker`.
pub(crate) async fn list_v1(call: &Call<'_>) -> Result<Response<Body>> {
    let request = Request::read(call)?;
    let marker = call.query_value("marker");
    let listing = request.run(call, marker.map(str::to_owned), None).await?;
    respond(call, &listing, &Version::One { marker })
}

fn respond(
    call: &Call<'_>,
    listing: &Listing<'_>,
    version: &Version<'_>,
) -> Result<Response<Body>> {
    let owner = call.verified.access_key();
    Ok(xml_response(
        StatusCode::OK,
        document::render(listing, version, owner)?,
    ))
}

/// What both versions share.
struct Request<'a> {
    prefix: &'a str,
    delimiter: Option<&'a str>,
    max_keys: usize,
    url: bool,
}

impl<'a> Request<'a> {
    fn read(call: &'a Call<'_>) -> Result<Self> {
        Ok(Self {
            prefix: call.query_value("prefix").unwrap_or(""),
            // An empty delimiter rolls nothing up.
            delimiter: call.query_value("delimiter").filter(|d| !d.is_empty()),
            max_keys: max_keys(call)?,
            url: url_encoded(call)?,
        })
    }

    fn scope<'s>(&'s self, call: &'s Call<'_>) -> Result<TokenScope<'s>> {
        Ok(TokenScope {
            bucket: call.bucket()?.as_str(),
            prefix: self.prefix,
            delimiter: self.delimiter,
        })
    }

    async fn run(
        &self,
        call: &Call<'_>,
        start_after: Option<String>,
        resume: Option<tessari_s3_core::objects::listing::Resume>,
    ) -> Result<Listing<'a>> {
        let spec = ListSpec {
            prefix: self.prefix.to_owned(),
            delimiter: self.delimiter.map(str::to_owned),
            max_keys: self.max_keys,
            start_after,
            resume,
        };
        let bucket = call.bucket()?;
        match call.state.storage().objects().list(bucket, spec).await? {
            Listed::Page(page) => Ok(Listing {
                bucket: bucket.as_str().to_owned(),
                prefix: self.prefix,
                delimiter: self.delimiter,
                max_keys: self.max_keys,
                url: self.url,
                page,
            }),
            Listed::NoSuchBucket => Err(Error::new(
                ErrorCode::NoSuchBucket,
                "the specified bucket does not exist",
            )),
        }
    }
}
