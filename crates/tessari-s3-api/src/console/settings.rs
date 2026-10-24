//! The settings the region's nodes share, managed here: `GET /api/v1/settings` lists them — a secret by name only —
//! and `PUT /api/v1/settings/{name}` changes one. Root only; a change needs a fresh sign-in and a reason, is checked
//! against this node's configuration before it is stored, and every attempt is recorded without the value.

use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use axum::http::HeaderMap;
use serde::{Deserialize, Serialize};
use tessari_s3_core::authz::{Action, Principal};
use tessari_s3_core::console::Session;
use tessari_s3_infrastructure::settings_keys::stored_setting;
use tessari_s3_storage::Error;
use tessari_s3_storage::actions::NewAction;
use tessari_s3_storage::keys::Passphrase;
use tessari_s3_storage::settings::SettingView;
use zeroize::Zeroizing;

use super::ConsoleState;
use super::access::{allow, recently_authenticated};
use super::actions::record;
use super::encryption::refused_passphrase;
use super::error::ConsoleError;
use super::input::{json, reason};

/// How the node checks a stored setting's new value: its whole configuration read with that one value changed.
pub type SettingsCheck =
    Arc<dyn Fn(&str, &str) -> Result<(), tessari_s3_infrastructure::Error> + Send + Sync>;

#[derive(Serialize)]
pub(super) struct SettingsView {
    region: String,
    settings: Vec<SettingEntry>,
}

#[derive(Serialize)]
struct SettingEntry {
    name: &'static str,
    scope: &'static str,
    class: &'static str,
    value: Option<String>,
    stored: bool,
    applies: &'static str,
    fixed: bool,
}

impl From<SettingView> for SettingEntry {
    fn from(view: SettingView) -> Self {
        Self {
            name: view.name,
            scope: if view.global { "global" } else { "region" },
            class: if view.secret { "secret" } else { "setting" },
            value: view.value,
            stored: view.stored,
            applies: applies(view.live),
            fixed: view.fixed,
        }
    }
}

const fn applies(live: bool) -> &'static str {
    if live { "live" } else { "start" }
}

#[derive(Deserialize)]
pub(super) struct ChangeRequest {
    value: String,
    expected: Option<String>,
    reason: Option<String>,
    passphrase: Option<String>,
}

#[derive(Serialize)]
pub(super) struct Changed {
    name: &'static str,
    applies: &'static str,
}

/// Every stored-class setting this node's region runs on.
pub(super) async fn list(
    State(state): State<ConsoleState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<SettingsView>, ConsoleError> {
    allow(&principal, &Action::Operate)?;
    let settings = state.storage().settings().view(state.region()).await?;
    Ok(Json(SettingsView {
        region: state.region().to_owned(),
        settings: settings.into_iter().map(SettingEntry::from).collect(),
    }))
}

/// Changes stored setting `name` against the value the operator read; a secret needs the keys passphrase.
pub(super) async fn change(
    State(state): State<ConsoleState>,
    Extension(session): Extension<Session>,
    Extension(principal): Extension<Principal>,
    Path(name): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Changed>, ConsoleError> {
    recently_authenticated(&state, &session)?;
    allow(&principal, &Action::Operate)?;
    let Some(stored) = stored_setting(&name) else {
        return Err(ConsoleError::not_found());
    };
    let ChangeRequest {
        value,
        expected,
        reason: given,
        passphrase,
    } = json(
        &headers,
        &body,
        "expected value, expected, reason and passphrase",
    )?;
    let why = reason(given, true)?;
    let (outcome, answer) =
        changed(&state, stored.key, &value, expected.as_deref(), passphrase).await;
    record(
        &state,
        NewAction {
            operator: session.subject.operator(),
            operation: "change_setting".to_owned(),
            target: format!("setting:{}", stored.key),
            reason: why,
            outcome: outcome.to_owned(),
        },
    )
    .await?;
    answer.map(|applies| {
        Json(Changed {
            name: stored.key,
            applies,
        })
    })
}

/// The change itself, with the outcome its record carries.
async fn changed(
    state: &ConsoleState,
    key: &'static str,
    value: &str,
    expected: Option<&str>,
    passphrase: Option<String>,
) -> (&'static str, Result<&'static str, ConsoleError>) {
    if stored_setting(key).is_some_and(|stored| stored.fixed) {
        return (
            "fixed",
            Err(ConsoleError::conflict(
                "setting_fixed",
                "this setting may not change once stored",
            )),
        );
    }
    let Some(check) = state.settings_check() else {
        return (
            "unmanaged",
            Err(ConsoleError::refused(
                "settings_unmanaged",
                "this console serves no node whose configuration could check the value",
            )),
        );
    };
    if let Err(refused) = check(key, value) {
        let why = match refused {
            tessari_s3_infrastructure::Error::InvalidConfig { reason, .. } => reason,
            _ => "the configuration does not accept this value",
        };
        return (
            "invalid",
            Err(ConsoleError::invalid("setting_invalid", why)),
        );
    }
    let passphrase = match passphrase.map(|text| Passphrase::new(Zeroizing::new(text))) {
        Some(Ok(passphrase)) => Some(passphrase),
        Some(Err(error)) => {
            let (outcome, refused) = refused_passphrase(error);
            return (outcome, Err(refused));
        }
        None => None,
    };
    let written = state
        .storage()
        .settings()
        .change(state.region(), key, expected, value, passphrase.as_ref())
        .await;
    match written {
        Ok(key) => (
            "changed",
            Ok(applies(
                tessari_s3_infrastructure::APPLIED_LIVE.contains(&key),
            )),
        ),
        Err(Error::FixedSetting { .. }) => (
            "fixed",
            Err(ConsoleError::conflict(
                "setting_fixed",
                "this setting may not change once stored",
            )),
        ),
        Err(Error::SettingChanged { .. }) => (
            "raced",
            Err(ConsoleError::conflict(
                "setting_changed",
                "someone changed this setting first; read it again",
            )),
        ),
        Err(Error::KeysSealed) => (
            "passphrase_required",
            Err(ConsoleError::invalid(
                "passphrase_required",
                "a secret is written with the keys passphrase",
            )),
        ),
        Err(error) => {
            let (outcome, refused) = refused_passphrase(error);
            (outcome, Err(refused))
        }
    }
}
