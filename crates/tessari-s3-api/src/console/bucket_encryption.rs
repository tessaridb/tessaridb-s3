//! A bucket's default encryption, through the console: who may configure the bucket sets it to the server's key, a
//! named key or none — and, beside a default, whether writes may bring their own key — with a reason that is recorded. As over S3, a default is set only when this node can seal under
//! it now — its keys unsealed and a named key held — so an operator learns at once that a write would be refused.

use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use tessari_s3_core::authz::policy::Context;
use tessari_s3_core::authz::{Action, Principal};
use tessari_s3_core::console::Session;
use tessari_s3_storage::Error;
use tessari_s3_storage::actions::NewAction;
use tessari_s3_storage::buckets::BucketEncryption;
use tessari_s3_storage::encryption::{KeyName, Sealing};

use super::ConsoleState;
use super::access::on_bucket;
use super::actions::record;
use super::buckets::bucket_name;
use super::error::ConsoleError;
use super::input::{json, reason};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SetEncryption {
    algorithm: Option<String>,
    key: Option<String>,
    customer_keys: Option<bool>,
    reason: Option<String>,
}

/// The sealing the operator asked for; `None` removes the default.
fn asked(algorithm: Option<&str>, key: Option<&str>) -> Result<Option<Sealing>, ConsoleError> {
    match (algorithm, key) {
        (Some("none"), None) => Ok(None),
        (Some("AES256"), None) => Ok(Some(Sealing::Server)),
        (Some("aws:kms"), Some(key)) => KeyName::new(key)
            .map(|name| Some(Sealing::Named(name)))
            .ok_or(ConsoleError::invalid(
                "invalid_key_name",
                "a key name is 1-64 lowercase letters, digits and hyphens",
            )),
        _ => Err(ConsoleError::invalid(
            "invalid_encryption",
            "algorithm is none, AES256, or aws:kms with a key",
        )),
    }
}

pub(super) async fn set(
    State(state): State<ConsoleState>,
    Extension(session): Extension<Session>,
    Extension(principal): Extension<Principal>,
    Extension(context): Extension<Context>,
    Path(bucket): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ConsoleError> {
    super::access::recently_authenticated(&state, &session)?;
    let SetEncryption {
        algorithm,
        key,
        customer_keys,
        reason: given,
    } = json(
        &headers,
        &body,
        "expected algorithm, key, customer_keys and reason",
    )?;
    let why = reason(given, true)?;
    let sealing = asked(algorithm.as_deref(), key.as_deref())?;
    let allow_customer_keys = customer_keys.unwrap_or(false);
    if allow_customer_keys && sealing.is_none() {
        return Err(ConsoleError::invalid(
            "invalid_encryption",
            "customer keys are allowed only beside a default",
        ));
    }
    let name = bucket_name(&bucket)?;
    on_bucket(
        &state,
        &principal,
        &context,
        &name,
        Action::ConfigureBucket,
        ("s3:PutEncryptionConfiguration", None),
    )
    .await?;
    let checked = match &sealing {
        Some(sealing) => match state.storage().keys().check(sealing) {
            Ok(()) => Ok(()),
            Err(Error::KeysSealed) => Err((
                "sealed",
                ConsoleError::conflict(
                    "keys_sealed",
                    "the encryption keys are sealed; unseal them first",
                ),
            )),
            Err(Error::UnknownKey { .. }) => Err((
                "unknown_key",
                ConsoleError::invalid("unknown_key", "no key of that name exists"),
            )),
            Err(error) => return Err(error.into()),
        },
        None => Ok(()),
    };
    let (outcome, answer) = match checked {
        Ok(()) => {
            let encryption = sealing.map(|sealing| BucketEncryption {
                sealing,
                bucket_key: false,
                allow_customer_keys,
            });
            if state
                .storage()
                .buckets()
                .set_encryption(&name, encryption.as_ref())
                .await?
            {
                ("done", Ok(StatusCode::NO_CONTENT.into_response()))
            } else {
                (
                    "no_such_bucket",
                    Err(ConsoleError::missing(
                        "no_such_bucket",
                        "there is no such bucket",
                    )),
                )
            }
        }
        Err((outcome, refused)) => (outcome, Err(refused)),
    };
    let mut asked_for = match (algorithm.as_deref(), key.as_deref()) {
        (Some(algorithm), Some(key)) => format!("{algorithm} {key}"),
        (Some(algorithm), None) => algorithm.to_owned(),
        _ => String::new(),
    };
    if allow_customer_keys {
        asked_for.push_str(" customer-keys");
    }
    record(
        &state,
        NewAction {
            operator: session.subject.operator(),
            operation: format!("set_encryption {asked_for}"),
            target: name.as_str().to_owned(),
            reason: why,
            outcome: outcome.to_owned(),
        },
    )
    .await?;
    answer
}
