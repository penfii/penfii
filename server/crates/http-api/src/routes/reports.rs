//! Sprint, velocity, schedule and status reports.

use axum::Json;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use penfii_core::project_schedule;
use penfii_core::reports;
use uuid::Uuid;

use crate::AppState;
use crate::error::ApiError;
use crate::extract::{Actor, Q};

use axum::Router;
use axum::routing::get;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/sprints/{id}/report", get(sprint_report))
        .route("/v1/projects/{id}/velocity", get(project_velocity))
        .route("/v1/projects/{id}/schedule", get(project_schedule))
        .route(
            "/v1/projects/{id}/status-summary",
            get(project_status_summary),
        )
}

/// The figures of one Sprint for the person asking.
async fn sprint_report(
    State(state): State<AppState>,
    Path(sprint_id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    Ok(Json(reports::sprint_report(&state.pool, user_id, sprint_id).await?).into_response())
}

#[derive(serde::Deserialize)]
struct VelocityQuery {
    limit: Option<i64>,
}

async fn project_velocity(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    Q(query): Q<VelocityQuery>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    Ok(
        Json(reports::velocity(&state.pool, user_id, project_id, query.limit).await?)
            .into_response(),
    )
}

async fn project_status_summary(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    Ok(Json(reports::status_summary(&state.pool, user_id, project_id).await?).into_response())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScheduleQuery {
    channel_id: Option<Uuid>,
    cursor: Option<String>,
    limit: Option<i64>,
}

/// The Tickets of a Project with their schedules and the blocking dependencies between them, for
/// the Gantt.
async fn project_schedule(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    Q(query): Q<ScheduleQuery>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    Ok(Json(
        project_schedule::schedule(
            &state.pool,
            user_id,
            project_id,
            query.channel_id,
            query.cursor.as_deref(),
            query.limit,
        )
        .await?,
    )
    .into_response())
}
