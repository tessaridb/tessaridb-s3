//! The SigV4 signing key chain, the string to sign, and the constant-time comparison.

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use zeroize::{Zeroize, ZeroizeOnDrop};

use super::canonical::{SignedRequest, canonical_request};
use super::encode::hex_lower;
use super::time::AmzDateTime;
use super::{AuthError, AuthResult};

/// The only SigV4 algorithm S3 uses.
pub const ALGORITHM: &str = "AWS4-HMAC-SHA256";

/// A secret access key. SigV4 is symmetric, so the server holds it to compute HMACs; it is scrubbed on drop and its
/// `Debug` shows nothing.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SecretKey(String);

impl SecretKey {
    /// Wraps a secret access key.
    #[must_use]
    pub const fn new(secret: String) -> Self {
        Self(secret)
    }
}

impl std::fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretKey(..)")
    }
}

/// The derived key for one (secret, date, region, `s3`) scope; scrubbed on drop.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SigningKey([u8; 32]);

impl std::fmt::Debug for SigningKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SigningKey(..)")
    }
}

impl SigningKey {
    /// Derives `HMAC(HMAC(HMAC(HMAC("AWS4" + secret, date), region), "s3"), "aws4_request")`.
    ///
    /// # Errors
    /// [`AuthError::SignatureMismatch`] if the HMAC cannot be keyed, which RFC 2104 rules out; it fails closed.
    pub fn derive(secret: &SecretKey, scope: &CredentialScope) -> AuthResult<Self> {
        let mut seed = Vec::with_capacity(secret.0.len().saturating_add(4));
        seed.extend_from_slice(b"AWS4");
        seed.extend_from_slice(secret.0.as_bytes());
        let date_key = hmac(&seed, scope.date.as_bytes());
        seed.zeroize();
        let region_key = hmac(&date_key?, scope.region.as_bytes())?;
        let service_key = hmac(&region_key, b"s3")?;
        Ok(Self(hmac(&service_key, b"aws4_request")?))
    }

    /// The raw HMAC of `message` under this key.
    ///
    /// # Errors
    /// As [`SigningKey::derive`].
    pub fn sign(&self, message: &[u8]) -> AuthResult<[u8; 32]> {
        hmac(&self.0, message)
    }
}

/// `YYYYMMDD/region/s3/aws4_request`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialScope {
    date: String,
    region: String,
}

impl CredentialScope {
    /// Parses the four scope components after the access key in a `Credential` value.
    ///
    /// # Errors
    /// [`AuthError::Malformed`] when the shape is wrong; [`AuthError::ScopeMismatch`] when the service or terminator
    /// is not `s3/aws4_request`.
    pub fn parse(date: &str, region: &str, service: &str, terminator: &str) -> AuthResult<Self> {
        if date.len() != 8 || !date.bytes().all(|b| b.is_ascii_digit()) || region.is_empty() {
            return Err(AuthError::Malformed {
                reason: "credential scope",
            });
        }
        if service != "s3" || terminator != "aws4_request" {
            return Err(AuthError::ScopeMismatch {
                reason: "service must be s3/aws4_request",
            });
        }
        Ok(Self {
            date: date.to_owned(),
            region: region.to_owned(),
        })
    }

    /// Checks the scope against the request time and the server's region.
    ///
    /// # Errors
    /// [`AuthError::ScopeMismatch`] for another date or region.
    pub fn check(&self, datetime: &AmzDateTime, server_region: &str) -> AuthResult<()> {
        if self.date != datetime.date() {
            return Err(AuthError::ScopeMismatch {
                reason: "date differs from the request date",
            });
        }
        if self.region != server_region {
            return Err(AuthError::ScopeMismatch {
                reason: "region is not this server's",
            });
        }
        Ok(())
    }

    /// The scope as it appears in the string to sign.
    #[must_use]
    pub fn to_line(&self) -> String {
        format!("{}/{}/s3/aws4_request", self.date, self.region)
    }
}

/// `AWS4-HMAC-SHA256\n<datetime>\n<scope>\n<hex sha256(canonical request)>`.
#[must_use]
pub fn string_to_sign(
    datetime: &AmzDateTime,
    scope: &CredentialScope,
    canonical_request: &str,
) -> String {
    format!(
        "{ALGORITHM}\n{}\n{}\n{}",
        datetime.as_str(),
        scope.to_line(),
        hex_lower(&Sha256::digest(canonical_request.as_bytes()))
    )
}

/// Compares a computed signature with the hex one the client sent, in constant time over the hex text.
///
/// # Errors
/// [`AuthError::SignatureMismatch`] when they differ.
pub fn check_signature(computed: &[u8; 32], supplied_hex: &str) -> AuthResult<()> {
    let computed_hex = hex_lower(computed);
    if bool::from(computed_hex.as_bytes().ct_eq(supplied_hex.as_bytes())) {
        Ok(())
    } else {
        Err(AuthError::SignatureMismatch)
    }
}

/// What a signature is checked over: the request, the parts the client declared, and who it claims to be.
#[derive(Debug)]
pub struct SignatureClaim<'a> {
    /// Query parameter left out of the canonical request (`X-Amz-Signature` for a presigned URL).
    pub exclude_query: Option<&'a str>,
    /// The client's SignedHeaders.
    pub signed_headers: &'a [String],
    /// The payload-hash line exactly as declared.
    pub payload_hash: &'a str,
    /// The request time.
    pub datetime: &'a AmzDateTime,
    /// The credential scope.
    pub scope: &'a CredentialScope,
    /// The hex signature the client sent.
    pub signature_hex: &'a str,
}

/// Rebuilds the canonical request and the string to sign, derives the key and checks the signature.
///
/// # Errors
/// Canonicalisation errors, or [`AuthError::SignatureMismatch`].
pub fn verify_signature(
    request: &SignedRequest<'_>,
    claim: &SignatureClaim<'_>,
    secret: &SecretKey,
) -> AuthResult<(SigningKey, [u8; 32])> {
    let canonical = canonical_request(
        request,
        claim.exclude_query,
        claim.signed_headers,
        claim.payload_hash,
    )?;
    let key = SigningKey::derive(secret, claim.scope)?;
    let computed = key.sign(string_to_sign(claim.datetime, claim.scope, &canonical).as_bytes())?;
    check_signature(&computed, claim.signature_hex)?;
    Ok((key, computed))
}

/// Parses `AK/YYYYMMDD/region/s3/aws4_request` into the access key and its scope.
///
/// # Errors
/// [`AuthError::Malformed`] or [`AuthError::ScopeMismatch`].
pub fn parse_credential(credential: &str) -> AuthResult<(String, CredentialScope)> {
    let parts: Vec<&str> = credential.split('/').collect();
    match parts.as_slice() {
        [access_key, date, region, service, terminator] if !access_key.is_empty() => Ok((
            (*access_key).to_owned(),
            CredentialScope::parse(date, region, service, terminator)?,
        )),
        _ => Err(AuthError::Malformed {
            reason: "credential",
        }),
    }
}

/// Splits a SignedHeaders value; the names must be lowercase.
///
/// # Errors
/// [`AuthError::Malformed`] for an empty list or a name with uppercase letters.
pub fn parse_signed_headers(value: &str) -> AuthResult<Vec<String>> {
    let names: Vec<String> = value.split(';').map(str::to_owned).collect();
    let well_formed = names
        .iter()
        .all(|name| !name.is_empty() && !name.bytes().any(|b| b.is_ascii_uppercase()));
    if well_formed {
        Ok(names)
    } else {
        Err(AuthError::Malformed {
            reason: "signed headers",
        })
    }
}

/// Whether `signature` is 64 lowercase hex digits.
#[must_use]
pub fn is_signature_hex(signature: &str) -> bool {
    signature.len() == 64
        && signature
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// HMAC-SHA256 of `message` under `key`. HMAC takes a key of any length (RFC 2104), so the keying error cannot
/// occur; if it ever did, the request is refused rather than signed under a substitute key.
fn hmac(key: &[u8], message: &[u8]) -> AuthResult<[u8; 32]> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).map_err(|_| AuthError::SignatureMismatch)?;
    mac.update(message);
    Ok(mac.finalize().into_bytes().into())
}
