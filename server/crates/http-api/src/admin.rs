//! Workspace administration, Ticket trash and purge, and the health probes.
//!
//! Every route here authorizes in core from the signed-in person's current membership; the HTTP layer
//! only parses, requires a recent sign-in for the destructive ones, and tells open sockets that a
//! person's access changed.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use penfii_core::{ChangeNotice, admin, ticket_lifecycle};
use serde::Deserialize;
use serde_json::json;
use std::time::Duration;
use uuid::Uuid;

use crate::AppState;
use crate::error::ApiError;
use crate::extract::{self, Actor, Q, RawJson, RecentActor};

use axum::Router;
use axum::routing::{delete, get, patch, post, put};

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/v1/workspaces/{id}/members", get(list_members))
        .route("/v1/workspaces/{id}/people", get(list_people))
        .route(
            "/v1/workspaces/{id}/members/{user_id}",
            patch(change_member_role).delete(remove_member),
        )
        .route(
            "/v1/workspaces/{id}/members/{user_id}/account",
            put(set_account_disabled),
        )
        .route(
            "/v1/workspaces/{id}/invitations/{invitation_id}",
            delete(revoke_invitation),
        )
        .route(
            "/v1/workspaces/{id}/teams/{team_id}/members",
            get(list_team_members),
        )
        .route(
            "/v1/workspaces/{id}/teams/{team_id}/members/{user_id}",
            put(add_team_member).delete(remove_team_member),
        )
        .route("/v1/workspaces/{id}/audit", get(list_audit))
        .route("/v1/projects/{id}/trash", get(project_trash))
        .route("/v1/tickets/{id}/commands/trash", post(trash_ticket))
        .route(
            "/v1/tickets/{id}/commands/restore-from-trash",
            post(restore_ticket_from_trash),
        )
        .route("/v1/tickets/{id}/commands/purge", post(purge_ticket))
}

fn access_changed(state: &AppState, user_id: Uuid) {
    state.publish([ChangeNotice::Access { user_id }]);
}

fn no_content() -> Response {
    StatusCode::NO_CONTENT.into_response()
}

pub async fn list_people(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    let items = admin::list_people(&state.pool, user_id, id).await?;
    Ok(Json(json!({ "items": items })).into_response())
}

pub async fn list_members(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    let items = admin::list_members(&state.pool, user_id, id).await?;
    Ok(Json(json!({ "items": items })).into_response())
}

#[derive(Deserialize)]
struct TeamMemberBody {
    role: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RoleBody {
    role: String,
}

pub async fn change_member_role(
    State(state): State<AppState>,
    Path((id, target)): Path<(Uuid, Uuid)>,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let body: RoleBody = extract::parse(raw)?;
    admin::change_member_role(&state.pool, user_id, id, target, &body.role).await?;
    Ok(no_content())
}

pub async fn remove_member(
    State(state): State<AppState>,
    Path((id, target)): Path<(Uuid, Uuid)>,
    RecentActor(user_id): RecentActor,
) -> Result<Response, ApiError> {
    admin::remove_member(&state.pool, user_id, id, target).await?;
    access_changed(&state, target);
    Ok(no_content())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AccountBody {
    disabled: bool,
}

pub async fn set_account_disabled(
    State(state): State<AppState>,
    Path((id, target)): Path<(Uuid, Uuid)>,
    RecentActor(user_id): RecentActor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let body: AccountBody = extract::parse(raw)?;
    admin::set_account_disabled(&state.pool, user_id, id, target, body.disabled).await?;
    if body.disabled {
        access_changed(&state, target);
    }
    Ok(no_content())
}

pub async fn list_invitations(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    let items = admin::list_invitations(&state.pool, user_id, id).await?;
    Ok(Json(json!({ "items": items })).into_response())
}

pub async fn revoke_invitation(
    State(state): State<AppState>,
    Path((id, invitation_id)): Path<(Uuid, Uuid)>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    admin::revoke_invitation(&state.pool, user_id, id, invitation_id).await?;
    Ok(no_content())
}

pub async fn list_team_members(
    State(state): State<AppState>,
    Path((id, team_id)): Path<(Uuid, Uuid)>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    let items = admin::list_team_members(&state.pool, user_id, id, team_id).await?;
    Ok(Json(json!({ "items": items })).into_response())
}

pub async fn add_team_member(
    State(state): State<AppState>,
    Path((id, team_id, target)): Path<(Uuid, Uuid, Uuid)>,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let body: TeamMemberBody = extract::parse(raw)?;
    admin::add_team_member(&state.pool, user_id, id, team_id, target, &body.role).await?;
    Ok(no_content())
}

pub async fn remove_team_member(
    State(state): State<AppState>,
    Path((id, team_id, target)): Path<(Uuid, Uuid, Uuid)>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    admin::remove_team_member(&state.pool, user_id, id, team_id, target).await?;
    Ok(no_content())
}

#[derive(Deserialize)]
pub struct AuditQuery {
    action: Option<String>,
    limit: Option<i64>,
    cursor: Option<String>,
}

pub async fn list_audit(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Q(query): Q<AuditQuery>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    let page = admin::list_audit(
        &state.pool,
        user_id,
        id,
        query.action.as_deref(),
        query.limit,
        query.cursor.as_deref(),
    )
    .await?;
    Ok(Json(page).into_response())
}

pub async fn trash_ticket(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    ticket_lifecycle::trash_ticket(&state.pool, user_id, id).await?;
    Ok(no_content())
}

pub async fn restore_ticket_from_trash(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    ticket_lifecycle::restore_from_trash(&state.pool, user_id, id).await?;
    Ok(no_content())
}

pub async fn purge_ticket(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    RecentActor(user_id): RecentActor,
) -> Result<Response, ApiError> {
    ticket_lifecycle::purge_ticket(&state.pool, state.objects.as_ref(), user_id, id).await?;
    Ok(no_content())
}

pub async fn project_trash(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    let items = ticket_lifecycle::list_trash(&state.pool, user_id, id).await?;
    Ok(Json(json!({ "items": items })).into_response())
}

/// Liveness: the process answers. It touches nothing else, so a database outage does not make an
/// orchestrator restart a healthy process.
pub async fn healthz() -> Response {
    Json(json!({"status": "ok"})).into_response()
}

/// Readiness: PostgreSQL answers (required) and, when search is configured, OpenSearch is not red
/// (optional: chat, work and docs keep working without it, so it only marks the instance `degraded`).
/// 503 when the database does not answer.
pub async fn readyz(State(state): State<AppState>) -> Response {
    let database = tokio::time::timeout(
        Duration::from_secs(2),
        sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(&state.pool),
    )
    .await
    .is_ok_and(|result| result.is_ok());
    let search = match &state.search {
        None => "disabled",
        Some(engine) => {
            let health = engine
                .indexer
                .client
                .request(
                    http::Method::GET,
                    "/_cluster/health",
                    None,
                    Duration::from_secs(2),
                )
                .await;
            match health {
                Ok((200, body)) if body["status"] != "red" => "ok",
                _ => "unavailable",
            }
        }
    };
    let status = if !database {
        "unavailable"
    } else if search == "unavailable" {
        "degraded"
    } else {
        "ok"
    };
    let code = if database {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        code,
        Json(json!({
            "status": status,
            "checks": {
                "database": if database { "ok" } else { "unavailable" },
                "search": search,
            }
        })),
    )
        .into_response()
}
