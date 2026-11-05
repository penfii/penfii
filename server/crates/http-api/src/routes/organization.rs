//! Teams and Departments.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use penfii_contracts::CreateTeamRequest;
use penfii_core::organization;
use uuid::Uuid;

use crate::AppState;
use crate::error::ApiError;
use crate::extract::{self, Actor, RawJson, present_or_null};

use axum::Router;
use axum::routing::{get, patch, post, put};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/workspaces/{id}/teams", post(create_team))
        .route("/v1/workspaces/{id}/teams/{team_id}", patch(update_team))
        .route(
            "/v1/workspaces/{id}/teams/{team_id}/archive",
            post(archive_team),
        )
        .route(
            "/v1/workspaces/{id}/teams/{team_id}/restore",
            post(restore_team),
        )
        .route(
            "/v1/workspaces/{id}/teams/{team_id}/projects",
            get(team_projects),
        )
        .route(
            "/v1/workspaces/{id}/departments",
            get(list_departments).post(create_department),
        )
        .route(
            "/v1/workspaces/{id}/departments/{department_id}",
            patch(update_department).delete(delete_department),
        )
        .route(
            "/v1/workspaces/{id}/teams/{team_id}/department",
            put(place_team),
        )
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateTeamBody {
    name: Option<String>,
    /// Absent leaves the description; `null` or an empty string clears it.
    #[serde(default, deserialize_with = "present_or_null")]
    description: Option<Option<String>>,
}

async fn update_team(
    State(state): State<AppState>,
    Path((id, team_id)): Path<(Uuid, Uuid)>,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let body: UpdateTeamBody = extract::parse(raw)?;
    let team = organization::update_team(
        &state.pool,
        user_id,
        id,
        team_id,
        body.name.as_deref(),
        body.description.as_ref().map(|text| text.as_deref()),
    )
    .await?;
    Ok(Json(team).into_response())
}

async fn archive_team(
    State(state): State<AppState>,
    Path((id, team_id)): Path<(Uuid, Uuid)>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    let team = organization::set_team_archived(&state.pool, user_id, id, team_id, true).await?;
    Ok(Json(team).into_response())
}

async fn restore_team(
    State(state): State<AppState>,
    Path((id, team_id)): Path<(Uuid, Uuid)>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    let team = organization::set_team_archived(&state.pool, user_id, id, team_id, false).await?;
    Ok(Json(team).into_response())
}

async fn team_projects(
    State(state): State<AppState>,
    Path((id, team_id)): Path<(Uuid, Uuid)>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    let items = organization::team_projects(&state.pool, user_id, id, team_id).await?;
    Ok(Json(serde_json::json!({ "items": items })).into_response())
}

async fn create_team(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let body: CreateTeamRequest = extract::parse(raw)?;
    let team = organization::create_team(
        &state.pool,
        user_id,
        id,
        &body.name,
        body.description.as_deref(),
        &body.members,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(team)).into_response())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DepartmentWrite {
    name: String,
    parent_id: Option<Uuid>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlaceTeamRequest {
    department_id: Option<Uuid>,
}

async fn list_departments(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    let items = organization::list_departments(&state.pool, user_id, id).await?;
    Ok(Json(serde_json::json!({ "items": items })).into_response())
}

async fn create_department(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let body: DepartmentWrite = extract::parse(raw)?;
    let department =
        organization::create_department(&state.pool, user_id, id, &body.name, body.parent_id)
            .await?;
    Ok((StatusCode::CREATED, Json(department)).into_response())
}

async fn update_department(
    State(state): State<AppState>,
    Path((id, department_id)): Path<(Uuid, Uuid)>,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let body: DepartmentWrite = extract::parse(raw)?;
    let department = organization::update_department(
        &state.pool,
        user_id,
        id,
        department_id,
        &body.name,
        body.parent_id,
    )
    .await?;
    Ok(Json(department).into_response())
}

async fn delete_department(
    State(state): State<AppState>,
    Path((id, department_id)): Path<(Uuid, Uuid)>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    organization::delete_department(&state.pool, user_id, id, department_id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn place_team(
    State(state): State<AppState>,
    Path((id, team_id)): Path<(Uuid, Uuid)>,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let body: PlaceTeamRequest = extract::parse(raw)?;
    let team =
        organization::place_team(&state.pool, user_id, id, team_id, body.department_id).await?;
    Ok(Json(team).into_response())
}
