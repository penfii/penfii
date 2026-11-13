//! The Workspace itself: server info, creation, settings and invitations.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use penfii_contracts::{
    AcceptInvitationRequest, CreateInvitationRequest, CreateWorkspaceRequest, ServerInfo,
    ServerLimits,
};
use penfii_core::organization;
use uuid::Uuid;

use crate::AppState;
use crate::admin;
use crate::error::ApiError;
use crate::extract::{Actor, Body, Session};

use axum::Router;
use axum::routing::{get, patch, post};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/server-info", get(server_info))
        .route(
            "/v1/workspaces",
            get(list_workspaces).post(create_workspace),
        )
        .route("/v1/workspaces/creation", get(workspace_creation))
        .route(
            "/v1/workspaces/{id}/invitations",
            get(admin::list_invitations).post(create_invitation),
        )
        .route(
            "/v1/workspaces/{id}/invitations/{invitation_id}/reissue",
            post(reissue_invitation),
        )
        .route("/v1/invitations/preview", post(preview_invitation))
        .route("/v1/invitations/accept", post(accept_invitation))
        .route("/v1/workspaces/{id}", patch(update_workspace))
}

async fn server_info(State(state): State<AppState>) -> Json<ServerInfo> {
    let mut features: Vec<String> = vec![
        "session".into(),
        "workspace".into(),
        "channel".into(),
        "dm".into(),
        "message".into(),
        "thread".into(),
        "reaction".into(),
        "read".into(),
        "directory".into(),
        "workflow".into(),
        "tickets".into(),
        "project-board".into(),
    ];
    if state.mcp.is_some() {
        features.push("mcp".into());
    }
    // The server sends e-mail: invitations are mailed as well as linked.
    if state.mailer.is_some() {
        features.push("email".into());
        features.push("password-reset".into());
    }
    Json(ServerInfo {
        api_version: "penfii.chat.v1".to_string(),
        features,
        limits: ServerLimits {
            message_body_max_bytes: state.limits.message_body_max_bytes,
            file_max_bytes: state.limits.file_max_bytes,
            session_ttl_seconds: state.limits.session_ttl_seconds,
        },
    })
}

async fn list_workspaces(
    State(state): State<AppState>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    Ok(Json(organization::list_workspaces(&state.pool, user_id).await?).into_response())
}

async fn workspace_creation(
    State(state): State<AppState>,
    Actor(_): Actor,
) -> Result<Response, ApiError> {
    let open = organization::workspace_creation_open(&state.pool).await?;
    Ok(Json(serde_json::json!({ "open": open })).into_response())
}

async fn create_workspace(
    State(state): State<AppState>,
    Actor(user_id): Actor,
    Body(body): Body<CreateWorkspaceRequest>,
) -> Result<Response, ApiError> {
    let workspace =
        organization::create_workspace(&state.pool, user_id, &body.name, &body.time_zone).await?;
    Ok((StatusCode::CREATED, Json(workspace)).into_response())
}

async fn create_invitation(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
    Body(body): Body<CreateInvitationRequest>,
) -> Result<Response, ApiError> {
    let invitation = organization::create_invitation(
        &state.pool,
        &state.limits,
        state.mailer.as_deref(),
        user_id,
        id,
        &body.email,
        &body.role,
        body.team_id,
    )
    .await?;
    // 201 for a new invitation, 200 when the e-mail's open invitation was re-issued.
    let status = if invitation.reissued {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((status, Json(invitation)).into_response())
}

/// The "send again" of the invitation list: a new link and expiry for an open invitation (the old
/// link stops working), mailed again when the server sends e-mail.
async fn reissue_invitation(
    State(state): State<AppState>,
    Path((id, invitation_id)): Path<(Uuid, Uuid)>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    let invitation = organization::reissue_invitation(
        &state.pool,
        &state.limits,
        state.mailer.as_deref(),
        user_id,
        id,
        invitation_id,
    )
    .await?;
    Ok(Json(invitation).into_response())
}

#[derive(serde::Deserialize)]
struct InvitationTokenRequest {
    token: String,
}

async fn preview_invitation(
    State(state): State<AppState>,
    Body(body): Body<InvitationTokenRequest>,
) -> Result<Response, ApiError> {
    let preview = organization::preview_invitation(&state.pool, &body.token).await?;
    Ok(Json(preview).into_response())
}

async fn accept_invitation(
    State(state): State<AppState>,
    session: Option<Session>,
    Body(body): Body<AcceptInvitationRequest>,
) -> Result<Response, ApiError> {
    let session = session.map(|Session(session)| session);
    let accepted = organization::accept_invitation(
        &state.pool,
        &body.token,
        &body.display_name,
        &body.password,
        session.as_ref(),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(accepted)).into_response())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateWorkspaceBody {
    name: Option<String>,
    time_zone: Option<String>,
}

async fn update_workspace(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
    Body(body): Body<UpdateWorkspaceBody>,
) -> Result<Response, ApiError> {
    let workspace = organization::update_workspace(
        &state.pool,
        user_id,
        id,
        body.name.as_deref(),
        body.time_zone.as_deref(),
    )
    .await?;
    Ok(Json(workspace).into_response())
}
