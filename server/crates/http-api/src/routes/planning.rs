//! Ticket order, saved Views, dependencies, Sprints and Milestones.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::NaiveDate;
use penfii_core::CoreError;
use penfii_core::planning::{
    self, MilestoneInput, MilestoneUpdate, Placement, SprintCommand, SprintInput, SprintUpdate,
};
use penfii_core::work::{self};
use serde_json::{Value, json};
use uuid::Uuid;

use super::key;
use crate::AppState;
use crate::error::ApiError;
use crate::extract::{self, Actor, RawJson, present_or_null};

use axum::Router;
use axum::routing::{delete, get, patch, post};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/tickets/{id}/commands/move", post(move_ticket))
        .route(
            "/v1/tickets/{id}/commands/assign-sprint",
            post(assign_sprint),
        )
        .route(
            "/v1/tickets/{id}/commands/set-milestones",
            post(set_milestones),
        )
        .route(
            "/v1/tickets/{id}/dependencies",
            get(ticket_dependencies).post(add_dependency),
        )
        .route("/v1/dependencies/{id}", delete(remove_dependency))
        .route(
            "/v1/dependencies/{id}/commands/waive",
            post(waive_dependency),
        )
        .route(
            "/v1/dependencies/{id}/commands/restore",
            post(restore_dependency),
        )
        .route(
            "/v1/workspaces/{id}/views",
            get(wide_views).post(create_wide_view),
        )
        .route(
            "/v1/projects/{id}/views",
            get(project_views).post(create_view),
        )
        .route("/v1/views/{id}", patch(update_view).delete(delete_view))
        .route("/v1/projects/{id}/planning", get(project_planning))
        .route("/v1/projects/{id}/sprints", post(create_sprint))
        .route("/v1/sprints/{id}", patch(update_sprint))
        .route("/v1/sprints/{id}/close-preview", get(sprint_close_preview))
        .route("/v1/sprints/{id}/results", get(sprint_results))
        .route("/v1/sprints/{id}/commands/{command}", post(sprint_command))
        .route("/v1/projects/{id}/milestones", post(create_milestone))
        .route("/v1/milestones/{id}", patch(update_milestone))
}

// Order, Views, dependencies, Sprints, Milestones -------------------------

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MoveTicketBody {
    expected_revision: Option<i64>,
    status_id: Option<Uuid>,
    after_id: Option<Uuid>,
    before_id: Option<Uuid>,
    position: Option<String>,
}

async fn move_ticket(
    State(state): State<AppState>,
    Path(ticket_id): Path<Uuid>,
    headers: HeaderMap,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let body: MoveTicketBody = extract::parse(raw)?;
    let placement = match (body.after_id, body.before_id, body.position.as_deref()) {
        (None, None, None) => Placement::Keep,
        (Some(id), None, None) => Placement::After(id),
        (None, Some(id), None) => Placement::Before(id),
        (None, None, Some("first")) => Placement::First,
        (None, None, Some("last")) => Placement::Last,
        _ => {
            return Err(ApiError::validation(
                "give one of afterId, beforeId or position (first, last)",
            ));
        }
    };
    let (ticket, change) = work::move_ticket(
        &state.pool,
        user_id,
        ticket_id,
        body.expected_revision,
        body.status_id,
        placement,
        &key(&headers)?,
    )
    .await?;
    state.publish(change);
    Ok(Json(ticket).into_response())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateViewBody {
    name: String,
    #[serde(default = "personal_scope")]
    scope: String,
    #[serde(default = "board_layout")]
    layout: String,
    #[serde(default)]
    definition: Value,
}

fn personal_scope() -> String {
    "PERSONAL".into()
}

fn board_layout() -> String {
    "BOARD".into()
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateWideViewBody {
    /// `TEAM` or `WORKSPACE`.
    target_kind: String,
    team_id: Option<Uuid>,
    name: String,
    #[serde(default = "list_layout")]
    layout: String,
    #[serde(default)]
    definition: Value,
}

fn list_layout() -> String {
    "LIST".into()
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct WideViewQuery {
    target_kind: String,
    team_id: Option<Uuid>,
}

fn wide_target(
    workspace_id: Uuid,
    kind: &str,
    team_id: Option<Uuid>,
) -> Result<planning::ViewTarget, ApiError> {
    match (kind, team_id) {
        ("TEAM", Some(team_id)) => Ok(planning::ViewTarget::Team {
            workspace_id,
            team_id,
        }),
        ("WORKSPACE", None) => Ok(planning::ViewTarget::Workspace(workspace_id)),
        _ => Err(ApiError::from(CoreError::validation(
            "targetKind must be TEAM with a teamId, or WORKSPACE",
        ))),
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateViewBody {
    expected_version: i64,
    name: Option<String>,
    layout: Option<String>,
    definition: Option<Value>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct VersionQuery {
    expected_version: i64,
}

async fn project_views(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    Ok(Json(
        planning::list_views(
            &state.pool,
            user_id,
            planning::ViewTarget::Project(project_id),
        )
        .await?,
    )
    .into_response())
}

async fn wide_views(
    State(state): State<AppState>,
    Path(workspace_id): Path<Uuid>,
    extract::Q(query): extract::Q<WideViewQuery>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    let target = wide_target(workspace_id, &query.target_kind, query.team_id)?;
    Ok(Json(planning::list_views(&state.pool, user_id, target).await?).into_response())
}

async fn create_wide_view(
    State(state): State<AppState>,
    Path(workspace_id): Path<Uuid>,
    headers: HeaderMap,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let body: CreateWideViewBody = extract::parse(raw)?;
    let target = wide_target(workspace_id, &body.target_kind, body.team_id)?;
    let definition = if body.definition.is_null() {
        json!({"group":"CATEGORY"})
    } else {
        body.definition
    };
    let view = planning::create_view(
        &state.pool,
        user_id,
        target,
        &body.name,
        "PERSONAL",
        &body.layout,
        definition,
        &key(&headers)?,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(view)).into_response())
}

async fn create_view(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    headers: HeaderMap,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let body: CreateViewBody = extract::parse(raw)?;
    let definition = if body.definition.is_null() {
        json!({})
    } else {
        body.definition
    };
    let view = planning::create_view(
        &state.pool,
        user_id,
        planning::ViewTarget::Project(project_id),
        &body.name,
        &body.scope,
        &body.layout,
        definition,
        &key(&headers)?,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(view)).into_response())
}

async fn update_view(
    State(state): State<AppState>,
    Path(view_id): Path<Uuid>,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let body: UpdateViewBody = extract::parse(raw)?;
    let view = planning::update_view(
        &state.pool,
        user_id,
        view_id,
        body.expected_version,
        body.name,
        body.layout,
        body.definition,
    )
    .await?;
    Ok(Json(view).into_response())
}

async fn delete_view(
    State(state): State<AppState>,
    Path(view_id): Path<Uuid>,
    extract::Q(query): extract::Q<VersionQuery>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    planning::delete_view(&state.pool, user_id, view_id, query.expected_version).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AddDependencyBody {
    predecessor_id: Uuid,
    #[serde(default = "blocks_kind")]
    kind: String,
}

fn blocks_kind() -> String {
    "BLOCKS".into()
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WaiverBody {
    reason: String,
}

async fn ticket_dependencies(
    State(state): State<AppState>,
    Path(ticket_id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    Ok(Json(planning::ticket_dependencies(&state.pool, user_id, ticket_id).await?).into_response())
}

async fn add_dependency(
    State(state): State<AppState>,
    Path(ticket_id): Path<Uuid>,
    headers: HeaderMap,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let body: AddDependencyBody = extract::parse(raw)?;
    let (value, change) = planning::add_dependency(
        &state.pool,
        user_id,
        ticket_id,
        body.predecessor_id,
        &body.kind,
        &key(&headers)?,
    )
    .await?;
    state.publish(change);
    Ok((StatusCode::CREATED, Json(value)).into_response())
}

async fn waive_dependency(
    State(state): State<AppState>,
    Path(dependency_id): Path<Uuid>,
    headers: HeaderMap,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let body: WaiverBody = extract::parse(raw)?;
    let (value, change) = planning::set_waiver(
        &state.pool,
        user_id,
        dependency_id,
        true,
        &body.reason,
        &key(&headers)?,
    )
    .await?;
    state.publish(change);
    Ok(Json(value).into_response())
}

async fn restore_dependency(
    State(state): State<AppState>,
    Path(dependency_id): Path<Uuid>,
    headers: HeaderMap,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let body: WaiverBody = extract::parse(raw)?;
    let (value, change) = planning::set_waiver(
        &state.pool,
        user_id,
        dependency_id,
        false,
        &body.reason,
        &key(&headers)?,
    )
    .await?;
    state.publish(change);
    Ok(Json(value).into_response())
}

async fn remove_dependency(
    State(state): State<AppState>,
    Path(dependency_id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    let change = planning::remove_dependency(&state.pool, user_id, dependency_id).await?;
    state.publish(change);
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn project_planning(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    Ok(Json(planning::project_planning(&state.pool, user_id, project_id).await?).into_response())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateSprintBody {
    name: String,
    goal: Option<String>,
    starts_on: NaiveDate,
    ends_on: NaiveDate,
    time_zone: Option<String>,
    estimate_unit: Option<String>,
}

async fn create_sprint(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    headers: HeaderMap,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let body: CreateSprintBody = extract::parse(raw)?;
    let sprint = planning::create_sprint(
        &state.pool,
        user_id,
        project_id,
        SprintInput {
            name: body.name,
            goal: body.goal,
            starts_on: body.starts_on,
            ends_on: body.ends_on,
            time_zone: body.time_zone,
            estimate_unit: body.estimate_unit,
        },
        &key(&headers)?,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(sprint)).into_response())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateSprintBody {
    expected_version: i64,
    name: Option<String>,
    #[serde(default, deserialize_with = "present_or_null")]
    goal: Option<Option<String>>,
    starts_on: Option<NaiveDate>,
    ends_on: Option<NaiveDate>,
    time_zone: Option<String>,
    estimate_unit: Option<String>,
}

async fn update_sprint(
    State(state): State<AppState>,
    Path(sprint_id): Path<Uuid>,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let body: UpdateSprintBody = extract::parse(raw)?;
    let sprint = planning::update_sprint(
        &state.pool,
        user_id,
        sprint_id,
        body.expected_version,
        SprintUpdate {
            name: body.name,
            goal: body.goal,
            starts_on: body.starts_on,
            ends_on: body.ends_on,
            time_zone: body.time_zone,
            estimate_unit: body.estimate_unit,
        },
    )
    .await?;
    Ok(Json(sprint).into_response())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct VersionBody {
    expected_version: i64,
    /// Closing only: the PLANNED/ACTIVE Sprint that takes the unfinished Tasks; none means backlog.
    #[serde(default)]
    carry_to_sprint_id: Option<Uuid>,
}

async fn sprint_command(
    State(state): State<AppState>,
    Path((sprint_id, command)): Path<(Uuid, String)>,
    headers: HeaderMap,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let body: VersionBody = extract::parse(raw)?;
    let command = match command.as_str() {
        "start" => SprintCommand::Start,
        "complete" => SprintCommand::Complete,
        "cancel" => SprintCommand::Cancel,
        _ => return Err(ApiError::validation("unknown Sprint command")),
    };
    let (sprint, changes) = planning::sprint_command(
        &state.pool,
        user_id,
        sprint_id,
        body.expected_version,
        command,
        body.carry_to_sprint_id,
        &key(&headers)?,
    )
    .await?;
    state.publish(changes);
    Ok(Json(sprint).into_response())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AssignSprintBody {
    sprint_id: Option<Uuid>,
}

async fn assign_sprint(
    State(state): State<AppState>,
    Path(ticket_id): Path<Uuid>,
    headers: HeaderMap,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let body: AssignSprintBody = extract::parse(raw)?;
    let (ticket, change) = planning::assign_sprint(
        &state.pool,
        user_id,
        ticket_id,
        body.sprint_id,
        &key(&headers)?,
    )
    .await?;
    state.publish(change);
    Ok(Json(ticket).into_response())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateMilestoneBody {
    name: String,
    description: Option<String>,
    target_on: Option<NaiveDate>,
}

async fn create_milestone(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    headers: HeaderMap,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let body: CreateMilestoneBody = extract::parse(raw)?;
    let milestone = planning::create_milestone(
        &state.pool,
        user_id,
        project_id,
        MilestoneInput {
            name: body.name,
            description: body.description,
            target_on: body.target_on,
        },
        &key(&headers)?,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(milestone)).into_response())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateMilestoneBody {
    expected_version: i64,
    name: Option<String>,
    #[serde(default, deserialize_with = "present_or_null")]
    description: Option<Option<String>>,
    #[serde(default, deserialize_with = "present_or_null")]
    target_on: Option<Option<NaiveDate>>,
    state: Option<String>,
}

async fn update_milestone(
    State(state): State<AppState>,
    Path(milestone_id): Path<Uuid>,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let body: UpdateMilestoneBody = extract::parse(raw)?;
    let milestone = planning::update_milestone(
        &state.pool,
        user_id,
        milestone_id,
        body.expected_version,
        MilestoneUpdate {
            name: body.name,
            description: body.description,
            target_on: body.target_on,
            state: body.state,
        },
    )
    .await?;
    Ok(Json(milestone).into_response())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetMilestonesBody {
    milestone_ids: Vec<Uuid>,
}

async fn set_milestones(
    State(state): State<AppState>,
    Path(ticket_id): Path<Uuid>,
    headers: HeaderMap,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let body: SetMilestonesBody = extract::parse(raw)?;
    let (ticket, change) = planning::set_milestones(
        &state.pool,
        user_id,
        ticket_id,
        body.milestone_ids,
        &key(&headers)?,
    )
    .await?;
    state.publish(change);
    Ok(Json(ticket).into_response())
}

async fn sprint_close_preview(
    State(state): State<AppState>,
    Path(sprint_id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    Ok(
        Json(planning::sprint_close_preview(&state.pool, user_id, sprint_id).await?)
            .into_response(),
    )
}

async fn sprint_results(
    State(state): State<AppState>,
    Path(sprint_id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    Ok(Json(planning::sprint_results(&state.pool, user_id, sprint_id).await?).into_response())
}
