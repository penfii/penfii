//! Projects: creation, settings, calendar, permissions, Team links, pins and designations.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use penfii_contracts::CreateProjectRequest;
use penfii_core::calendar;
use penfii_core::organization;
use penfii_core::personal;
use penfii_core::platform::ChangeNotice;
use penfii_core::project_access;
use penfii_core::work::{self};
use serde_json::json;
use uuid::Uuid;

use crate::AppState;
use crate::error::ApiError;
use crate::extract::{self, Actor, Body};

use axum::Router;
use axum::routing::{delete, get, patch, post, put};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/workspaces/{id}/projects", post(create_project))
        .route("/v1/workspaces/{id}/project-prefix", get(project_prefix))
        .route("/v1/projects/{id}", patch(update_project))
        .route(
            "/v1/projects/{id}/calendar",
            get(project_calendar).put(put_project_calendar),
        )
        .route("/v1/projects/{id}/pins", get(project_pins).post(add_pin))
        .route("/v1/pins/{id}", delete(remove_pin))
        .route(
            "/v1/projects/{id}/members/{user_id}/designation",
            put(set_designation),
        )
        .route("/v1/projects/{id}/members", get(project_members))
        .route("/v1/projects/{id}/ticket-types", get(project_ticket_types))
        .route("/v1/projects/{id}/permissions", get(project_permissions))
        .route(
            "/v1/projects/{id}/grants/{user_id}",
            put(set_project_grant).delete(remove_project_grant),
        )
        .route(
            "/v1/projects/{id}/teams/{team_id}",
            put(link_project_team).delete(unlink_project_team),
        )
        .route(
            "/v1/projects/{id}/teams/{team_id}/members",
            get(project_team_members),
        )
        .route(
            "/v1/projects/{id}/teams/{team_id}/participation",
            put(set_project_team_participation),
        )
        .route(
            "/v1/projects/{id}/private-channels",
            get(project_private_channels),
        )
}

/// The Project's private channels for its admin, readable or not (names and counts, no messages).
async fn project_private_channels(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    let items =
        penfii_core::conversation::project_private_channels(&state.pool, user_id, id).await?;
    Ok(Json(serde_json::json!({ "items": items })).into_response())
}

async fn project_members(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    Ok(Json(work::project_members(&state.pool, user_id, project_id).await?).into_response())
}

async fn project_ticket_types(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    Ok(Json(work::project_ticket_types(&state.pool, user_id, project_id).await?).into_response())
}

async fn create_project(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
    Body(body): Body<CreateProjectRequest>,
) -> Result<Response, ApiError> {
    let project = organization::create_project(
        &state.pool,
        user_id,
        id,
        &body.name,
        &body.team_ids,
        body.ticket_prefix.as_deref(),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(project)).into_response())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateProjectBody {
    estimate_unit: String,
}

/// Project settings that a Project admin changes after creation. Today: the estimate unit.
async fn update_project(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    Actor(user_id): Actor,
    Body(body): Body<UpdateProjectBody>,
) -> Result<Response, ApiError> {
    let project = organization::update_project_estimate_unit(
        &state.pool,
        user_id,
        project_id,
        &body.estimate_unit,
    )
    .await?;
    state.publish([ChangeNotice::Project { project_id }]);
    Ok(Json(project).into_response())
}

async fn project_calendar(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    Ok(Json(calendar::get_calendar(&state.pool, user_id, project_id).await?).into_response())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PutCalendarBody {
    /// 0 creates the calendar; otherwise the version the person read.
    expected_version: i64,
    time_zone: String,
    definition: penfii_contracts::CalendarDefinition,
}

async fn put_project_calendar(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    Actor(user_id): Actor,
    Body(body): Body<PutCalendarBody>,
) -> Result<Response, ApiError> {
    let saved = calendar::put_calendar(
        &state.pool,
        user_id,
        project_id,
        body.expected_version,
        &body.time_zone,
        body.definition,
    )
    .await?;
    state.publish([ChangeNotice::Project { project_id }]);
    let status = if body.expected_version == 0 {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(saved)).into_response())
}

#[derive(serde::Deserialize)]
struct PrefixQuery {
    name: Option<String>,
    prefix: Option<String>,
}

async fn project_prefix(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    extract::Q(query): extract::Q<PrefixQuery>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    Ok(Json(
        organization::prefix_check(
            &state.pool,
            user_id,
            id,
            query.name.as_deref(),
            query.prefix.as_deref(),
        )
        .await?,
    )
    .into_response())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AddPinBody {
    ticket_id: Option<Uuid>,
    url: Option<String>,
    title: Option<String>,
}

async fn project_pins(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    Ok(Json(personal::list_pins(&state.pool, user_id, project_id).await?).into_response())
}

async fn add_pin(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    Actor(user_id): Actor,
    Body(body): Body<AddPinBody>,
) -> Result<Response, ApiError> {
    let pins = personal::add_pin(
        &state.pool,
        user_id,
        project_id,
        body.ticket_id,
        body.url,
        body.title,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(pins)).into_response())
}

async fn remove_pin(
    State(state): State<AppState>,
    Path(pin_id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    personal::remove_pin(&state.pool, user_id, pin_id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DesignationBody {
    can_waive_dependencies: bool,
    can_manage_pins: bool,
}

async fn set_designation(
    State(state): State<AppState>,
    Path((project_id, member_id)): Path<(Uuid, Uuid)>,
    Actor(user_id): Actor,
    Body(body): Body<DesignationBody>,
) -> Result<Response, ApiError> {
    let (waive, pins) = personal::set_designation(
        &state.pool,
        user_id,
        project_id,
        member_id,
        body.can_waive_dependencies,
        body.can_manage_pins,
    )
    .await?;
    Ok(
        Json(json!({"userId":member_id,"canWaiveDependencies":waive,"canManagePins":pins}))
            .into_response(),
    )
}

async fn project_permissions(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    Ok(Json(personal::permissions(&state.pool, user_id, id).await?).into_response())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RoleBody {
    pub role: Option<String>,
}

async fn set_project_grant(
    State(state): State<AppState>,
    Path((id, target)): Path<(Uuid, Uuid)>,
    Actor(user_id): Actor,
    Body(body): Body<RoleBody>,
) -> Result<Response, ApiError> {
    let role = body
        .role
        .ok_or_else(|| penfii_core::CoreError::validation("role is required"))?;
    project_access::set_grant(&state.pool, user_id, id, target, &role).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn remove_project_grant(
    State(state): State<AppState>,
    Path((id, target)): Path<(Uuid, Uuid)>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    let notices = project_access::remove_grant(&state.pool, user_id, id, target).await?;
    state.publish(notices);
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn link_project_team(
    State(state): State<AppState>,
    Path((id, team_id)): Path<(Uuid, Uuid)>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    project_access::link_team(&state.pool, user_id, id, team_id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn unlink_project_team(
    State(state): State<AppState>,
    Path((id, team_id)): Path<(Uuid, Uuid)>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    let notices = project_access::unlink_team(&state.pool, user_id, id, team_id).await?;
    state.publish(notices);
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn project_team_members(
    State(state): State<AppState>,
    Path((id, team_id)): Path<(Uuid, Uuid)>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    let items = project_access::team_roster(&state.pool, user_id, id, team_id).await?;
    Ok(Json(items).into_response())
}

/// `{"role": "EDITOR"}` lets the Team's members into the Project with that role; `{"role": null}` ends it.
async fn set_project_team_participation(
    State(state): State<AppState>,
    Path((id, team_id)): Path<(Uuid, Uuid)>,
    Actor(user_id): Actor,
    Body(body): Body<RoleBody>,
) -> Result<Response, ApiError> {
    let notices = project_access::set_team_participation(
        &state.pool,
        user_id,
        id,
        team_id,
        body.role.as_deref(),
    )
    .await?;
    state.publish(notices);
    Ok(StatusCode::NO_CONTENT.into_response())
}
