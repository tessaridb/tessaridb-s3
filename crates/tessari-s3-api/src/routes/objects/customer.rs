//! Server-side encryption with a customer-provided key (SSE-C) on the wire: the three headers that bring the key, and
//! the two a response answers with. The key itself is never answered, logged or named in an error.

use axum::http::{HeaderMap, HeaderValue};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use md5::{Digest, Md5};
use subtle::ConstantTimeEq;
use tessari_s3_storage::encryption::{CustomerKey, CustomerSeal};
use tessari_s3_storage::objects::{Content, StoredObject};
use tessari_s3_types::ErrorCode;
use zeroize::Zeroizing;

use crate::pipeline::call::Call;
use crate::{Error, Result};

pub(crate) const ALGORITHM: &str = "x-amz-server-side-encryption-customer-algorithm";
pub(crate) const KEY: &str = "x-amz-server-side-encryption-customer-key";
pub(crate) const KEY_MD5: &str = "x-amz-server-side-encryption-customer-key-md5";

/// A customer's key as one request brought it, with the base64 MD5 that names it in answers.
#[derive(Debug)]
pub(crate) struct Customer {
    pub(crate) key: CustomerKey,
    pub(crate) md5: String,
}

fn invalid(message: &'static str) -> Error {
    Error::new(ErrorCode::InvalidArgument, message)
}

/// The customer's key `headers` bring, if any: all three headers, the algorithm AES256, the key 256 bits in
/// base64, and the MD5 the key's.
///
/// # Errors
/// `InvalidEncryptionAlgorithmError` for another algorithm; `InvalidArgument` for a missing header, a key that is
/// not 256 bits, or an MD5 that is not the key's.
pub(crate) fn read_customer(headers: &HeaderMap) -> Result<Option<Customer>> {
    let text = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
    };
    let (algorithm, key, md5) = match (text(ALGORITHM), text(KEY), text(KEY_MD5)) {
        (None, None, None) => return Ok(None),
        (None, _, _) => {
            return Err(invalid(
                "a customer-provided key needs x-amz-server-side-encryption-customer-algorithm",
            ));
        }
        (_, None, _) => {
            return Err(invalid(
                "a customer-provided key needs x-amz-server-side-encryption-customer-key",
            ));
        }
        (_, _, None) => {
            return Err(invalid(
                "a customer-provided key needs x-amz-server-side-encryption-customer-key-MD5",
            ));
        }
        (Some(algorithm), Some(key), Some(md5)) => (algorithm, key, md5),
    };
    if algorithm != "AES256" {
        return Err(Error::new(
            ErrorCode::InvalidEncryptionAlgorithmError,
            "the encryption algorithm for a customer-provided key must be AES256",
        ));
    }
    let decoded = Zeroizing::new(STANDARD.decode(key).unwrap_or_default());
    let bytes: Zeroizing<[u8; 32]> = Zeroizing::new(
        decoded
            .as_slice()
            .try_into()
            .map_err(|_| invalid("the customer-provided key is not a 256-bit key in base64"))?,
    );
    let digest = Md5::digest(bytes.as_slice());
    let declared = STANDARD.decode(md5).unwrap_or_default();
    if !bool::from(digest.as_slice().ct_eq(&declared)) {
        return Err(invalid(
            "the MD5 of the customer-provided key does not match the one supplied",
        ));
    }
    Ok(Some(Customer {
        key: CustomerKey::new(*bytes),
        md5: STANDARD.encode(digest),
    }))
}

/// Refuses a customer's key that arrived in the clear: anyone on the path has seen it.
///
/// # Errors
/// `InvalidRequest` when `call` did not arrive over TLS.
pub(crate) fn over_tls(call: &Call<'_>) -> Result<()> {
    if call.secure {
        return Ok(());
    }
    Err(Error::new(
        ErrorCode::InvalidRequest,
        "requests specifying server-side encryption with a customer-provided key must be made over a secure connection",
    ))
}

/// Admits a write sealed under `customer`'s key — over TLS, into a bucket whose default encryption allows customer
/// keys — and answers the fresh seal its data key is wrapped under.
///
/// # Errors
/// `InvalidRequest` in the clear; `NoSuchBucket`; `AccessDenied` for a bucket that does not allow customer keys;
/// the metadata store's refusal or outage.
pub(crate) async fn admit(call: &Call<'_>, customer: &Customer) -> Result<CustomerSeal> {
    over_tls(call)?;
    let bucket = call
        .state
        .storage()
        .buckets()
        .get(call.bucket()?)
        .await?
        .ok_or_else(|| {
            Error::new(
                ErrorCode::NoSuchBucket,
                "the specified bucket does not exist",
            )
        })?;
    if !bucket
        .encryption
        .is_some_and(|encryption| encryption.allow_customer_keys)
    {
        return Err(Error::new(
            ErrorCode::AccessDenied,
            "this bucket does not allow server-side encryption with a customer-provided key",
        ));
    }
    Ok(CustomerSeal::new(&customer.key)?)
}

/// The refusal of a read that brought no key for data sealed under a customer's key.
pub(crate) fn key_required() -> Error {
    Error::new(
        ErrorCode::InvalidRequest,
        "the object was stored using server-side encryption with a customer-provided key; provide that key to read it",
    )
}

/// The seal of the customer's key `object` is sealed under: its data's, or its first sealed part's.
pub(crate) fn customer_seal(object: &StoredObject) -> Option<&CustomerSeal> {
    match &object.content {
        Content::Data(_) => object.encryption.as_ref()?.customer.as_ref(),
        Content::Parts(multipart) => multipart
            .parts
            .iter()
            .find_map(|part| part.encryption.as_ref()?.customer.as_ref()),
    }
}

/// Admits a read of `object` bringing `customer`'s key, or none, as [`admit_key`] admits it against the object's seal.
///
/// # Errors
/// As [`admit_key`].
pub(crate) fn admit_read(
    call: &Call<'_>,
    object: &StoredObject,
    customer: Option<&Customer>,
) -> Result<()> {
    admit_key(call, customer_seal(object), customer)
}

/// Admits a request bringing `customer`'s key, or none, to data sealed under `seal`, or under none: the key the seal
/// was made for, or none for data sealed without one — compared through the seal's verifier, before any byte is
/// opened or written.
///
/// # Errors
/// `InvalidRequest` for a key in the clear, a missing key, or a key for data sealed without one; `AccessDenied` for
/// another key.
pub(crate) fn admit_key(
    call: &Call<'_>,
    seal: Option<&CustomerSeal>,
    customer: Option<&Customer>,
) -> Result<()> {
    if customer.is_some() {
        over_tls(call)?;
    }
    match (seal, customer) {
        (None, None) => Ok(()),
        (Some(_), None) => Err(key_required()),
        (None, Some(_)) => Err(Error::new(
            ErrorCode::InvalidRequest,
            "the encryption parameters are not applicable to this object",
        )),
        (Some(seal), Some(customer)) => {
            if seal.matches(&customer.key)? {
                Ok(())
            } else {
                Err(tessari_s3_storage::Error::CustomerKeyMismatch.into())
            }
        }
    }
}

/// Sets the response headers that name the customer's key: its algorithm and its MD5, never the key.
pub(crate) fn answer_customer(headers: &mut HeaderMap, customer: &Customer) {
    headers.insert(ALGORITHM, HeaderValue::from_static("AES256"));
    if let Ok(value) = HeaderValue::from_str(&customer.md5) {
        headers.insert(KEY_MD5, value);
    }
}

#[cfg(test)]
#[path = "customer_tests.rs"]
mod tests;
