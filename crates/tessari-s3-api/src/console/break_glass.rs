//! The root credential beside an identity provider is the emergency path, and it is made deliberate and loud: the
//! operator says why, the sign-in is announced to the configured webhook as it happens, and it is in the action record
//! before any session exists. An alert that cannot be delivered never bars the path — it is logged and named in the
//! record — while a record that cannot be written refuses the sign-in, because an unrecorded emergency is the one thing
//! this path must not allow.

use std::net::IpAddr;

use axum::body::Bytes;
use serde::Serialize;
use tessari_s3_infrastructure::{OidcSettings, WebhookClient};
use tessari_s3_storage::actions::NewAction;

use super::ConsoleState;
use super::error::ConsoleError;
use super::input::reason;

const OPERATION: &str = "break_glass_sign_in";

#[derive(Serialize)]
struct Alert<'a> {
    event: &'static str,
    key_id: &'a str,
    reason: &'a str,
    client: String,
    expires: i64,
}

/// The client and URL a root sign-in is announced to, when the identity provider's settings name a webhook.
pub(super) fn alert(
    settings: Option<&OidcSettings>,
) -> Result<Option<(WebhookClient, String)>, tessari_s3_infrastructure::Error> {
    settings
        .and_then(|settings| settings.break_glass_webhook.clone())
        .map(|url| WebhookClient::new().map(|client| (client, url)))
        .transpose()
}

/// Admits a root sign-in that would end at `expires`: requires `given` as its reason, announces it, records it.
pub(super) async fn admit(
    state: &ConsoleState,
    key_id: &str,
    given: Option<String>,
    client: IpAddr,
    expires: i64,
) -> Result<(), ConsoleError> {
    let why = reason(given, true)?.unwrap_or_default();
    let delivered = match state.break_glass_alert() {
        Some((webhook, url)) => {
            let alert = Alert {
                event: OPERATION,
                key_id,
                reason: &why,
                client: client.to_string(),
                expires,
            };
            let body = serde_json::to_vec(&alert).map_err(|_| ConsoleError::internal())?;
            webhook.post(url, Bytes::from(body)).await
        }
        None => true,
    };
    if !delivered {
        tracing::error!(client = %client, key_id, "a break-glass sign-in could not be announced");
    }
    let action = NewAction {
        operator: key_id.to_owned(),
        operation: OPERATION.to_owned(),
        target: key_id.to_owned(),
        reason: Some(why),
        outcome: if delivered { "done" } else { "alert_failed" }.to_owned(),
    };
    if let Err(error) = state.storage().actions().record(&action).await {
        tracing::error!(error = %error, client = %client, key_id, "a break-glass sign-in was refused: not recorded");
        return Err(ConsoleError::unavailable());
    }
    tracing::warn!(client = %client, key_id, reason = ?action.reason, "break-glass sign-in");
    Ok(())
}
