//! A Project's workflow states.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use penfii_core::work::{self, StateInput, StateUpdate};
use serde_json::json;
use uuid::Uuid;

use crate::AppState;
use crate::error::ApiError;
use crate::extract::{self, Actor, RawJson, present_or_null};

use axum::Router;
use axum::routing::{get, patch, post};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/projects/{id}/workflow", get(project_workflow))
        .route(
            "/v1/projects/{id}/workflow/states",
            post(create_workflow_state),
        )
        .route(
            "/v1/projects/{id}/workflow/states/{state_id}",
            patch(update_workflow_state).delete(delete_workflow_state),
        )
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateStateBody {
    expected_workflow_version: i64,
    name: String,
    category: String,
    position: i32,
    #[serde(default)]
    is_initial: bool,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateStateBody {
    expected_workflow_version: i64,
    name: Option<String>,
    category: Option<String>,
    position: Option<i32>,
    is_initial: Option<bool>,
    #[serde(default, deserialize_with = "present_or_null")]
    wip_limit: Option<Option<i32>>,
}

async fn project_workflow(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    let (version, states) = work::workflow(&state.pool, project_id, user_id).await?;
    Ok(Json(json!({"projectId":project_id,"version":version,"states":states})).into_response())
}

async fn create_workflow_state(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    Actor(actor_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let body: CreateStateBody = extract::parse(raw)?;
    let (version, state_row) = work::create_state(
        &state.pool,
        actor_id,
        project_id,
        body.expected_workflow_version,
        StateInput {
            name: body.name,
            category: body.category,
            position: body.position,
            is_initial: body.is_initial,
        },
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"version":version,"state":state_row})),
    )
        .into_response())
}

async fn update_workflow_state(
    State(state): State<AppState>,
    Path((project_id, state_id)): Path<(Uuid, Uuid)>,
    Actor(actor_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let body: UpdateStateBody = extract::parse(raw)?;
    let (version, state_row) = work::update_state(
        &state.pool,
        actor_id,
        project_id,
        state_id,
        body.expected_workflow_version,
        StateUpdate {
            name: body.name,
            category: body.category,
            position: body.position,
            is_initial: body.is_initial,
            wip_limit: body.wip_limit,
        },
    )
    .await?;
    Ok(Json(json!({"version":version,"state":state_row})).into_response())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeleteStateQuery {
    expected_workflow_version: i64,
}

async fn delete_workflow_state(
    State(state): State<AppState>,
    Path((project_id, state_id)): Path<(Uuid, Uuid)>,
    axum::extract::Query(query): axum::extract::Query<DeleteStateQuery>,
    Actor(actor_id): Actor,
) -> Result<Response, ApiError> {
    let version = work::delete_state(
        &state.pool,
        actor_id,
        project_id,
        state_id,
        query.expected_workflow_version,
    )
    .await?;
    Ok(Json(json!({"version":version})).into_response())
}
