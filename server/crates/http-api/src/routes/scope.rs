//! The Workspace-wide work dataset and bulk Ticket changes.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use penfii_core::scope;
use serde_json::Value;
use uuid::Uuid;

use super::key;
use crate::AppState;
use crate::error::ApiError;
use crate::extract::{self, Actor, RawJson};

use axum::Router;
use axum::routing::{get, post};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/workspaces/{id}/work", get(scope_dataset))
        .route("/v1/workspaces/{id}/tickets/bulk", post(bulk_tickets))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScopeQuery {
    scope: String,
    team_id: Option<Uuid>,
    #[serde(default)]
    lens: Option<String>,
    /// Comma-separated Project ids that narrow the scope before the cap.
    #[serde(default)]
    projects: Option<String>,
}

async fn scope_dataset(
    State(state): State<AppState>,
    Path(workspace_id): Path<Uuid>,
    extract::Q(query): extract::Q<ScopeQuery>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    let target = match (query.scope.as_str(), query.team_id) {
        ("TEAM", Some(team_id)) => scope::ScopeTarget::Team {
            workspace_id,
            team_id,
        },
        ("WORKSPACE", None) => scope::ScopeTarget::Workspace(workspace_id),
        _ => {
            return Err(ApiError::validation(
                "scope must be TEAM with a teamId, or WORKSPACE",
            ));
        }
    };
    let lens = match query.lens.as_deref() {
        None | Some("ALL") => scope::Lens::All,
        Some("ASSIGNED") => scope::Lens::Assigned,
        Some("CREATED") => scope::Lens::Created,
        Some("WATCHING") => scope::Lens::Watching,
        Some(_) => {
            return Err(ApiError::validation(
                "lens must be ALL, ASSIGNED, CREATED or WATCHING",
            ));
        }
    };
    let only: Vec<Uuid> = match query.projects.as_deref() {
        None | Some("") => Vec::new(),
        Some(list) => list
            .split(',')
            .map(|id| id.trim().parse::<Uuid>())
            .collect::<Result<_, _>>()
            .map_err(|_| ApiError::validation("projects must be a list of ids"))?,
    };
    Ok(
        Json(scope::scope_dataset(&state.pool, user_id, target, lens, &only).await?)
            .into_response(),
    )
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BulkBody {
    ticket_ids: Vec<Uuid>,
    change: Value,
}

fn bulk_change_from(value: &Value) -> Result<scope::BulkChange, ApiError> {
    let ids = |name: &str| -> Result<Vec<Uuid>, ApiError> {
        serde_json::from_value(value.get(name).cloned().unwrap_or(Value::Null))
            .map_err(|_| ApiError::validation(format!("{name} must be a list of ids")))
    };
    let optional_id = |name: &str| -> Result<Option<Uuid>, ApiError> {
        serde_json::from_value(value.get(name).cloned().unwrap_or(Value::Null))
            .map_err(|_| ApiError::validation(format!("{name} must be an id or null")))
    };
    match value.get("type").and_then(Value::as_str) {
        Some("STATUS") => Ok(scope::BulkChange::Status(
            optional_id("statusId")?.ok_or_else(|| ApiError::validation("statusId is required"))?,
        )),
        Some("ASSIGNEE") => Ok(scope::BulkChange::Assignee(optional_id("assigneeId")?)),
        Some("PRIORITY") => Ok(scope::BulkChange::Priority(
            value
                .get("priority")
                .and_then(Value::as_str)
                .map(str::to_owned),
        )),
        Some("ADD_LABELS") => Ok(scope::BulkChange::AddLabels(ids("labelIds")?)),
        Some("REMOVE_LABELS") => Ok(scope::BulkChange::RemoveLabels(ids("labelIds")?)),
        Some("SPRINT") => Ok(scope::BulkChange::Sprint(optional_id("sprintId")?)),
        Some("ADD_MILESTONES") => Ok(scope::BulkChange::AddMilestones(ids("milestoneIds")?)),
        Some("REMOVE_MILESTONES") => Ok(scope::BulkChange::RemoveMilestones(ids("milestoneIds")?)),
        _ => Err(ApiError::validation("unknown bulk change type")),
    }
}

async fn bulk_tickets(
    State(state): State<AppState>,
    Path(workspace_id): Path<Uuid>,
    headers: HeaderMap,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let body: BulkBody = extract::parse(raw)?;
    let change = bulk_change_from(&body.change)?;
    let (result, changes) = scope::bulk_change(
        &state.pool,
        user_id,
        workspace_id,
        body.ticket_ids,
        change,
        &key(&headers)?,
    )
    .await?;
    state.publish(changes);
    Ok(Json(result).into_response())
}
