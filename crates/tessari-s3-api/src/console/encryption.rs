//! The encryption keys of this node: `GET /api/v1/encryption`, `POST …/unseal`, `POST …/seal`, and `POST …/keys`,
//! which creates a key objects can be sealed under by name (`aws:kms`). Root only; changing
//! anything needs a fresh sign-in. Each node holds its own keys, so an unseal here unseals this node alone. Every
//! attempt is recorded with how it ended — never with the passphrase, which is dropped once the vault has it.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, State};
use axum::http::HeaderMap;
use serde::{Deserialize, Serialize};
use tessari_s3_core::authz::{Action, Principal};
use tessari_s3_core::console::Session;
use tessari_s3_storage::Error;
use tessari_s3_storage::actions::NewAction;
use tessari_s3_storage::encryption::KeyName;
use tessari_s3_storage::keys::{KeyState, Passphrase};
use zeroize::Zeroizing;

use super::ConsoleState;
use super::access::{allow, recently_authenticated};
use super::actions::record;
use super::error::ConsoleError;
use super::input::{json, reason};

/// What the record of an act names as its target.
const TARGET: &str = "encryption keys";

#[derive(Serialize)]
pub(super) struct KeysView {
    /// `uninitialised`, `sealed` or `unsealed`.
    state: &'static str,
    /// The key new data keys are wrapped under, when unsealed.
    active: Option<String>,
    /// How many keys this node holds, when unsealed.
    keys: Option<usize>,
    /// The names of the keys operators created, when unsealed.
    named: Vec<String>,
}

impl From<KeyState> for KeysView {
    fn from(state: KeyState) -> Self {
        match state {
            KeyState::Uninitialised => Self {
                state: "uninitialised",
                active: None,
                keys: None,
                named: Vec::new(),
            },
            KeyState::Sealed => Self {
                state: "sealed",
                active: None,
                keys: None,
                named: Vec::new(),
            },
            KeyState::Unsealed {
                active,
                keys,
                named,
            } => Self {
                state: "unsealed",
                active: Some(active),
                keys: Some(keys),
                named,
            },
        }
    }
}

#[derive(Deserialize)]
pub(super) struct UnsealRequest {
    passphrase: String,
    reason: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct CreateRequest {
    passphrase: String,
    name: String,
    reason: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct SealRequest {
    reason: Option<String>,
}

async fn recorded(
    state: &ConsoleState,
    session: &Session,
    operation: &str,
    why: Option<String>,
    outcome: &str,
) -> Result<(), ConsoleError> {
    record(
        state,
        NewAction {
            operator: session.subject.operator(),
            operation: operation.to_owned(),
            target: TARGET.to_owned(),
            reason: why,
            outcome: outcome.to_owned(),
        },
    )
    .await
}

pub(super) async fn state(
    State(state): State<ConsoleState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<KeysView>, ConsoleError> {
    allow(&principal, &Action::Operate)?;
    Ok(Json(state.storage().keys().state().await?.into()))
}

pub(super) async fn unseal(
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
    let opened = match Passphrase::new(Zeroizing::new(passphrase)) {
        Ok(passphrase) => state.storage().keys().unseal(&passphrase).await,
        Err(error) => Err(error),
    };
    let (outcome, answer) = match opened {
        Ok(keys) => ("unsealed", Ok(Json(keys.into()))),
        Err(error) => {
            let (outcome, refused) = refused_passphrase(error);
            (outcome, Err(refused))
        }
    };
    recorded(&state, &session, "unseal_keys", why, outcome).await?;
    answer
}

/// What a refused passphrase is answered with, and recorded as.
fn refused_passphrase(error: Error) -> (&'static str, ConsoleError) {
    match error {
        Error::WeakPassphrase { reason } => (
            "weak_passphrase",
            ConsoleError::invalid("weak_passphrase", reason),
        ),
        Error::WrongPassphrase => (
            "wrong_passphrase",
            ConsoleError::invalid(
                "wrong_passphrase",
                "the keys vault did not open with this passphrase",
            ),
        ),
        Error::PassphraseThrottled => ("throttled", ConsoleError::rate_limited()),
        other => ("failed", other.into()),
    }
}

/// Creates a key objects can then be sealed under by its name. The passphrase opens the keys vault for the one
/// write and is dropped.
pub(super) async fn create(
    State(state): State<ConsoleState>,
    Extension(session): Extension<Session>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<KeysView>, ConsoleError> {
    recently_authenticated(&state, &session)?;
    allow(&principal, &Action::Operate)?;
    let CreateRequest {
        passphrase,
        name,
        reason: given,
    } = json(&headers, &body, "expected passphrase, name and reason")?;
    let why = reason(given, false)?;
    let Some(name) = KeyName::new(&name) else {
        return Err(ConsoleError::invalid(
            "bad_key_name",
            "a key name is 1 to 64 of a-z, 0-9 and -",
        ));
    };
    let created = match Passphrase::new(Zeroizing::new(passphrase)) {
        Ok(passphrase) => {
            state
                .storage()
                .keys()
                .create_named(&passphrase, &name)
                .await
        }
        Err(error) => Err(error),
    };
    let (outcome, answer) = match created {
        Ok(keys) => ("created", Ok(Json(keys.into()))),
        Err(Error::KeyExists { .. }) => (
            "exists",
            Err(ConsoleError::conflict(
                "key_exists",
                "a key of this name exists; keys are never replaced",
            )),
        ),
        Err(error) => {
            let (outcome, refused) = refused_passphrase(error);
            (outcome, Err(refused))
        }
    };
    recorded(
        &state,
        &session,
        &format!("create_key {name}"),
        why,
        outcome,
    )
    .await?;
    answer
}

pub(super) async fn seal(
    State(state): State<ConsoleState>,
    Extension(session): Extension<Session>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<KeysView>, ConsoleError> {
    recently_authenticated(&state, &session)?;
    allow(&principal, &Action::Operate)?;
    let SealRequest { reason: given } = json(&headers, &body, "expected reason")?;
    let why = reason(given, false)?;
    state.storage().keys().seal();
    recorded(&state, &session, "seal_keys", why, "sealed").await?;
    Ok(Json(state.storage().keys().state().await?.into()))
}
