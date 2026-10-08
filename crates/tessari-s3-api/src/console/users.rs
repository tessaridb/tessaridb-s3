//! Users through the console: listed, created and disabled by the same user service the server uses. Who may manage
//! whom is the evaluator's answer on the user as it is — or, for a creation, as it would be — so a space admin reaches
//! only the plain members of its own space and nobody reaches themselves. Every change is recorded with its reason.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use tessari_s3_core::authz::{
    Action, Decision, Principal, Role, SpaceName, UserName, UserResource, Visible, authorize,
};
use tessari_s3_core::console::Session;
use tessari_s3_storage::actions::NewAction;
use tessari_s3_storage::users::{NewUser, User, UserCreated};

use super::ConsoleState;
use super::access::allow;
use super::actions::record;
use super::error::ConsoleError;
use super::input::{json, reason};

/// The evaluator's view of `user`.
fn resource(user: &User) -> UserResource {
    UserResource {
        name: user.name.as_str().to_owned(),
        space: user.space.clone(),
        role: user.role,
        operator: user.operator,
        cluster_viewer: user.cluster_viewer,
    }
}

/// The user called `name`, once `principal` may manage it. A user the caller cannot manage answers alike whether it
/// exists or not; only those who operate the store are told that it does not.
pub(super) async fn managed(
    state: &ConsoleState,
    principal: &Principal,
    name: &str,
) -> Result<User, ConsoleError> {
    let name = UserName::new(name).ok_or(ConsoleError::invalid(
        "invalid_user_name",
        "user names are 1-63 lowercase letters, digits and inner hyphens",
    ))?;
    match state.storage().users().get(&name).await? {
        Some(user) => {
            allow(principal, &Action::ManageUser(resource(&user)))?;
            Ok(user)
        }
        None if authorize(principal, &Action::Operate) == Decision::Allow => Err(
            ConsoleError::missing("no_such_user", "there is no such user"),
        ),
        None => Err(ConsoleError::forbidden()),
    }
}

const fn role_text(role: Role) -> &'static str {
    match role {
        Role::SpaceAdmin => "space_admin",
        Role::Member => "member",
    }
}

#[derive(Serialize)]
struct UserView {
    name: String,
    space: String,
    role: &'static str,
    create_buckets: bool,
    operator: bool,
    cluster_viewer: bool,
    disabled: bool,
    created: String,
}

impl From<User> for UserView {
    fn from(user: User) -> Self {
        Self {
            name: user.name.as_str().to_owned(),
            space: user.space.as_str().to_owned(),
            role: role_text(user.role),
            create_buckets: user.create_buckets,
            operator: user.operator,
            cluster_viewer: user.cluster_viewer,
            disabled: user.disabled,
            created: user.created.iso8601_millis(),
        }
    }
}

#[derive(Serialize)]
pub(super) struct Users {
    users: Vec<UserView>,
}

/// Operators see every space's users; a space admin its own space's; a member nobody's.
pub(super) async fn list(
    State(state): State<ConsoleState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<Users>, ConsoleError> {
    let visible = principal.visible();
    let needed = match &visible {
        Visible::All => Action::Operate,
        Visible::Space(space) => Action::ManageSpace(space.clone()),
    };
    allow(&principal, &needed)?;
    let users = state.storage().users().list(&visible).await?;
    Ok(Json(Users {
        users: users.into_iter().map(UserView::from).collect(),
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum RoleInput {
    SpaceAdmin,
    Member,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Create {
    name: String,
    space: String,
    role: RoleInput,
    #[serde(default)]
    create_buckets: bool,
    #[serde(default)]
    operator: bool,
    #[serde(default)]
    cluster_viewer: bool,
    reason: Option<String>,
}

pub(super) async fn create(
    State(state): State<ConsoleState>,
    Extension(session): Extension<Session>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ConsoleError> {
    let input: Create = json(
        &headers,
        &body,
        "expected name, space, role (space_admin or member), the optional flags and reason",
    )?;
    let why = reason(input.reason, true)?;
    let new = NewUser {
        name: UserName::new(&input.name).ok_or(ConsoleError::invalid(
            "invalid_user_name",
            "user names are 1-63 lowercase letters, digits and inner hyphens",
        ))?,
        space: SpaceName::new(&input.space).ok_or(ConsoleError::invalid(
            "invalid_space_name",
            "space names are 1-63 lowercase letters, digits and inner hyphens",
        ))?,
        role: match input.role {
            RoleInput::SpaceAdmin => Role::SpaceAdmin,
            RoleInput::Member => Role::Member,
        },
        create_buckets: input.create_buckets,
        operator: input.operator,
        cluster_viewer: input.cluster_viewer,
    };
    // The user as it would be is what is judged, so nobody creates authority they could not manage.
    allow(
        &principal,
        &Action::ManageUser(UserResource {
            name: new.name.as_str().to_owned(),
            space: new.space.clone(),
            role: new.role,
            operator: new.operator,
            cluster_viewer: new.cluster_viewer,
        }),
    )?;
    let created = state.storage().users().create(&new).await?;
    let outcome = match created {
        UserCreated::Created(_) => "done",
        UserCreated::Exists => "exists",
        UserCreated::NoSuchSpace => "no_such_space",
    };
    record(
        &state,
        NewAction {
            operator: session.key_id,
            operation: "create_user".to_owned(),
            target: new.name.as_str().to_owned(),
            reason: why,
            outcome: outcome.to_owned(),
        },
    )
    .await?;
    match created {
        UserCreated::Created(user) => {
            Ok((StatusCode::CREATED, Json(UserView::from(user))).into_response())
        }
        UserCreated::Exists => Err(ConsoleError::conflict(
            "user_exists",
            "a user of that name already exists",
        )),
        UserCreated::NoSuchSpace => Err(ConsoleError::missing(
            "no_such_space",
            "there is no such space",
        )),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Disable {
    disabled: bool,
    reason: Option<String>,
}

/// Disables or enables a user; its keys stop or start resolving within the principal cache's window.
pub(super) async fn set_disabled(
    State(state): State<ConsoleState>,
    Extension(session): Extension<Session>,
    Extension(principal): Extension<Principal>,
    Path(name): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ConsoleError> {
    let Disable {
        disabled,
        reason: given,
    } = json(&headers, &body, "expected disabled and reason")?;
    let why = reason(given, true)?;
    let user = managed(&state, &principal, &name).await?;
    let changed = state
        .storage()
        .users()
        .set_disabled(&user.name, disabled)
        .await?;
    record(
        &state,
        NewAction {
            operator: session.key_id,
            operation: if disabled {
                "disable_user"
            } else {
                "enable_user"
            }
            .to_owned(),
            target: user.name.as_str().to_owned(),
            reason: why,
            outcome: if changed { "done" } else { "no_such_user" }.to_owned(),
        },
    )
    .await?;
    if changed {
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Err(ConsoleError::missing(
            "no_such_user",
            "there is no such user",
        ))
    }
}
