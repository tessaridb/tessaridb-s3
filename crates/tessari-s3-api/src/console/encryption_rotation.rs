//! Rotating the server key: `POST /api/v1/encryption/rotate` makes a new server key the one new data keys are wrapped
//! under, and `POST …/destroy` removes an older one from the keys vault once the re-wrap has left no record naming it.
//! Root only, a fresh sign-in, the keys passphrase for the one vault write; every attempt is recorded.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, State};
use axum::http::HeaderMap;
use serde::Deserialize;
use tessari_s3_core::authz::{Action, Principal};
use tessari_s3_core::console::Session;
use tessari_s3_storage::Error;
use tessari_s3_storage::keys::Passphrase;
use zeroize::Zeroizing;

use super::ConsoleState;
use super::access::{allow, recently_authenticated};
use super::encryption::{KeysView, UnsealRequest, recorded, refused_passphrase};
use super::error::ConsoleError;
use super::input::{json, reason};

#[derive(Deserialize)]
pub(super) struct DestroyRequest {
    passphrase: String,
    key: String,
    reason: Option<String>,
}

/// Rotates the server key: the next one is created and new data keys are wrapped under it on this node at once;
/// every older key still opens what it sealed. The passphrase opens the keys vault for the one write and is dropped.
pub(super) async fn rotate(
    State(state): State<ConsoleState>,
    Extension(session): Extension<Session>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<KeysView>, ConsoleError> {
    recently_authenticated(&state, &session)?;
    allow(&principal, &Action::Operate)?;
    let UnsealRequest {
        passphrase,
        reason: given,
    } = json(&headers, &body, "expected passphrase and reason")?;
    let why = reason(given, false)?;
    let rotated = match Passphrase::new(Zeroizing::new(passphrase)) {
        Ok(passphrase) => state.storage().keys().rotate(&passphrase).await,
        Err(error) => Err(error),
    };
    let (outcome, answer) = match rotated {
        Ok(keys) => ("rotated", Ok(Json(keys.into()))),
        Err(Error::KeyExists { .. }) => (
            "raced",
            Err(ConsoleError::conflict(
                "rotation_raced",
                "another rotation took this key's number; reload and look again",
            )),
        ),
        Err(error) => {
            let (outcome, refused) = refused_passphrase(error);
            (outcome, Err(refused))
        }
    };
    recorded(&state, &session, "rotate_server_key", why, outcome).await?;
    answer
}

/// Destroys an older server key once no record names it: the count is taken at the act, and a key some record
/// still needs is refused rather than removed. The key leaves the live vault; copies of the vault taken before keep it
/// until they expire.
pub(super) async fn destroy(
    State(state): State<ConsoleState>,
    Extension(session): Extension<Session>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<KeysView>, ConsoleError> {
    recently_authenticated(&state, &session)?;
    allow(&principal, &Action::Operate)?;
    let DestroyRequest {
        passphrase,
        key,
        reason: given,
    } = json(&headers, &body, "expected passphrase, key and reason")?;
    let why = reason(given, false)?;
    let storage = state.storage();
    let refused_in_use = || {
        (
            "in_use",
            Err(ConsoleError::conflict(
                "key_in_use",
                "records still name this key; destroy it once the re-wrap has reached them all",
            )),
        )
    };
    // `Err(())`: records still name the key, and nothing was asked of the vault.
    let destroyed = match Passphrase::new(Zeroizing::new(passphrase)) {
        // The active key is refused by the destroy itself; any other is counted first, at the act.
        Ok(passphrase) => match storage.keys().ring().map(|ring| ring.active().id() == key) {
            Ok(false) => match storage.objects().remaining(&key).await {
                Ok(0) => Ok(storage.keys().destroy(&passphrase, &key).await),
                Ok(_) => Err(()),
                Err(error) => Ok(Err(error)),
            },
            Ok(true) => Ok(Err(Error::KeyActive { id: key.clone() })),
            Err(error) => Ok(Err(error)),
        },
        Err(error) => Ok(Err(error)),
    };
    let (outcome, answer) = match destroyed {
        Err(()) => refused_in_use(),
        Ok(Ok(keys)) => ("destroyed", Ok(Json(keys.into()))),
        Ok(Err(Error::KeyActive { .. })) => (
            "active",
            Err(ConsoleError::conflict(
                "key_active",
                "new data keys are wrapped under this key; rotate first",
            )),
        ),
        Ok(Err(Error::UnknownKey { .. })) => (
            "unknown",
            Err(ConsoleError::invalid(
                "not_an_older_key",
                "only an older server key this node holds can be destroyed",
            )),
        ),
        Ok(Err(error)) => {
            let (outcome, refused) = refused_passphrase(error);
            (outcome, Err(refused))
        }
    };
    recorded(
        &state,
        &session,
        &format!("destroy_server_key {key}"),
        why,
        outcome,
    )
    .await?;
    answer
}
