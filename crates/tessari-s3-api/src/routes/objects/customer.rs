//! Server-side encryption with a customer-provided key (SSE-C) on the wire: the three headers that bring the key, and
//! the two a response answers with. The key itself is never answered, logged or named in an error.

use axum::http::{HeaderMap, HeaderValue};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use md5::{Digest, Md5};
use subtle::ConstantTimeEq;
use tessari_s3_storage::encryption::{CustomerKey, CustomerSeal};
use tessari_s3_storage::objects::StoredObject;
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

const SOURCE_ALGORITHM: &str = "x-amz-copy-source-server-side-encryption-customer-algorithm";
const SOURCE_KEY: &str = "x-amz-copy-source-server-side-encryption-customer-key";
const SOURCE_KEY_MD5: &str = "x-amz-copy-source-server-side-encryption-customer-key-md5";

fn invalid(message: String) -> Error {
    Error::new(ErrorCode::InvalidArgument, message)
}

/// The customer's key `headers` bring for the data a request writes or reads, if any.
///
/// # Errors
/// As [`read_trio`].
pub(crate) fn read_customer(headers: &HeaderMap) -> Result<Option<Customer>> {
    read_trio(headers, [ALGORITHM, KEY, KEY_MD5])
}

/// The customer's key `headers` bring for a copy's source, if any.
///
/// # Errors
/// `NotImplemented` for another copy-source encryption header; as [`read_trio`].
pub(crate) fn read_source_customer(headers: &HeaderMap) -> Result<Option<Customer>> {
    let names = [SOURCE_ALGORITHM, SOURCE_KEY, SOURCE_KEY_MD5];
    if let Some(other) = headers.keys().map(|name| name.as_str()).find(|name| {
        name.starts_with("x-amz-copy-source-server-side-encryption") && !names.contains(name)
    }) {
        return Err(Error::new(
            ErrorCode::NotImplemented,
            format!("{other} is not implemented"),
        ));
    }
    read_trio(headers, names)
}

/// The key the three headers `names` (algorithm, key, key MD5) bring, if any: all three, the algorithm AES256, the
/// key 256 bits in base64, and the MD5 the key's.
///
/// # Errors
/// `InvalidEncryptionAlgorithmError` for another algorithm; `InvalidArgument` for a missing header, a key that is
/// not 256 bits, or an MD5 that is not the key's.
fn read_trio(headers: &HeaderMap, names: [&str; 3]) -> Result<Option<Customer>> {
    let text = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
    };
    let [algorithm_name, key_name, md5_name] = names;
    let (algorithm, key, md5) = match (text(algorithm_name), text(key_name), text(md5_name)) {
        (None, None, None) => return Ok(None),
        (Some(algorithm), Some(key), Some(md5)) => (algorithm, key, md5),
        (algorithm, key, _) => {
            let missing = match (algorithm, key) {
                (None, _) => algorithm_name,
                (_, None) => key_name,
                _ => md5_name,
            };
            return Err(invalid(format!("a customer-provided key needs {missing}")));
        }
    };
    if algorithm != "AES256" {
        return Err(Error::new(
            ErrorCode::InvalidEncryptionAlgorithmError,
            "the encryption algorithm for a customer-provided key must be AES256",
        ));
    }
    let decoded = Zeroizing::new(STANDARD.decode(key).unwrap_or_default());
    let bytes: Zeroizing<[u8; 32]> =
        Zeroizing::new(decoded.as_slice().try_into().map_err(|_| {
            invalid("the customer-provided key is not a 256-bit key in base64".to_owned())
        })?);
    let digest = Md5::digest(bytes.as_slice());
    let declared = STANDARD.decode(md5).unwrap_or_default();
    if !bool::from(digest.as_slice().ct_eq(&declared)) {
        return Err(invalid(
            "the MD5 of the customer-provided key does not match the one supplied".to_owned(),
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

/// Admits a read of `object` bringing `customer`'s key, or none, as [`admit_key`] admits it against the object's seal.
///
/// # Errors
/// As [`admit_key`].
pub(crate) fn admit_read(
    call: &Call<'_>,
    object: &StoredObject,
    customer: Option<&Customer>,
) -> Result<()> {
    admit_key(call, object.customer_seal(), customer)
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
