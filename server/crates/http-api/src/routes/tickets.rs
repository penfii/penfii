//! Tickets: the Board, Labels, the Ticket commands, comments, revisions and activity.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, NaiveDate, Utc};
use penfii_core::plan_fields;
use penfii_core::work::{self, TicketInput, TicketUpdate};
use serde_json::Value;
use uuid::Uuid;

use super::key;
use crate::AppState;
use crate::error::ApiError;
use crate::extract::{self, Actor, RawJson, present_or_null};

use axum::Router;
use axum::routing::{get, patch, post};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/projects/{id}/board", get(project_board))
        .route(
            "/v1/projects/{id}/labels",
            get(project_labels).post(create_project_label),
        )
        .route(
            "/v1/projects/{id}/tickets",
            get(project_tickets).post(create_ticket),
        )
        .route("/v1/projects/{id}/activity", get(project_activity))
        .route("/v1/tickets/{id}", get(get_ticket).patch(update_ticket))
        .route(
            "/v1/tickets/{id}/comments",
            get(ticket_comments).post(create_ticket_comment),
        )
        .route(
            "/v1/tickets/{id}/comments/{commentId}",
            patch(edit_ticket_comment).delete(delete_ticket_comment),
        )
        .route(
            "/v1/tickets/{id}/commands/change-status",
            post(change_ticket_status),
        )
        .route("/v1/tickets/{id}/revisions", get(ticket_revisions))
        .route(
            "/v1/tickets/{id}/revisions/{revision_no}",
            get(ticket_revision_detail),
        )
        .route("/v1/tickets/{id}/children", get(ticket_children))
        .route("/v1/tickets/{id}/activity", get(ticket_activity))
        .route("/v1/tickets/{id}/commands/restore", post(restore_ticket))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateTicketBody {
    title: String,
    #[serde(default)]
    description: Value,
    #[serde(default = "default_hierarchy")]
    hierarchy_kind: String,
    parent_id: Option<Uuid>,
    channel_id: Option<Uuid>,
    status_id: Option<Uuid>,
    assignee_id: Option<Uuid>,
    priority: Option<String>,
    due_on: Option<NaiveDate>,
    #[serde(default)]
    label_ids: Vec<Uuid>,
    type_id: Option<Uuid>,
    estimate: Option<EstimateBody>,
    schedule: Option<ScheduleBody>,
    #[serde(default)]
    file_ids: Vec<Uuid>,
}

/// An estimate as sent: a decimal string and, optionally, the unit it is written in.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct EstimateBody {
    value: String,
    unit: Option<String>,
}

impl From<EstimateBody> for plan_fields::EstimateInput {
    fn from(body: EstimateBody) -> Self {
        Self {
            value: body.value,
            unit: body.unit,
        }
    }
}

/// A schedule as sent. A `plannedEndAt` echoed back from a read is ignored: the end is derived.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScheduleBody {
    planned_start_at: Option<DateTime<Utc>>,
    duration_minutes: Option<i64>,
    #[serde(default)]
    is_schedule_milestone: bool,
}

impl From<ScheduleBody> for plan_fields::ScheduleInput {
    fn from(body: ScheduleBody) -> Self {
        Self {
            planned_start_at: body.planned_start_at,
            duration_minutes: body.duration_minutes,
            is_schedule_milestone: body.is_schedule_milestone,
        }
    }
}

fn default_hierarchy() -> String {
    "TASK".into()
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateTicketBody {
    expected_revision: i64,
    title: Option<String>,
    description: Option<Value>,
    #[serde(default, deserialize_with = "present_or_null")]
    assignee_id: Option<Option<Uuid>>,
    #[serde(default, deserialize_with = "present_or_null")]
    priority: Option<Option<String>>,
    #[serde(default, deserialize_with = "present_or_null")]
    due_on: Option<Option<NaiveDate>>,
    label_ids: Option<Vec<Uuid>>,
    #[serde(default, deserialize_with = "present_or_null")]
    parent_id: Option<Option<Uuid>>,
    #[serde(default, deserialize_with = "present_or_null")]
    type_id: Option<Option<Uuid>>,
    #[serde(default, deserialize_with = "present_or_null")]
    estimate: Option<Option<EstimateBody>>,
    #[serde(default, deserialize_with = "present_or_null")]
    schedule: Option<Option<ScheduleBody>>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RestoreTicketBody {
    expected_revision: i64,
    source_revision: i64,
    fields: Vec<String>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageQuery {
    limit: Option<i64>,
    before: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChangeStatusBody {
    expected_revision: i64,
    status_id: Uuid,
}

#[derive(serde::Deserialize)]
struct EditCommentBody {
    body: Value,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateCommentBody {
    body: Value,
    #[serde(default)]
    file_ids: Vec<Uuid>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateProjectLabelBody {
    name: String,
    color: Option<String>,
}

async fn project_board(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    Ok(Json(work::board(&state.pool, project_id, user_id).await?).into_response())
}

async fn project_labels(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    Ok(Json(work::project_labels(&state.pool, user_id, project_id).await?).into_response())
}

async fn create_project_label(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let body: CreateProjectLabelBody = extract::parse(raw)?;
    let label =
        work::create_project_label(&state.pool, user_id, project_id, &body.name, body.color)
            .await?;
    Ok((StatusCode::CREATED, Json(label)).into_response())
}

async fn project_tickets(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    let board = work::board(&state.pool, project_id, user_id).await?;
    let tickets = board
        .columns
        .into_iter()
        .flat_map(|column| column.tickets)
        .collect::<Vec<_>>();
    Ok(Json(tickets).into_response())
}

async fn create_ticket(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    headers: HeaderMap,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let body: CreateTicketBody = extract::parse(raw)?;
    let (ticket, change) = work::create_ticket(
        &state.pool,
        user_id,
        project_id,
        TicketInput {
            title: body.title,
            description: body.description,
            hierarchy_kind: body.hierarchy_kind,
            parent_id: body.parent_id,
            channel_id: body.channel_id,
            status_id: body.status_id,
            assignee_id: body.assignee_id,
            priority: body.priority,
            due_on: body.due_on,
            label_ids: body.label_ids,
            type_id: body.type_id,
            estimate: body.estimate.map(Into::into),
            schedule: body.schedule.map(Into::into),
            file_ids: body.file_ids,
        },
        &key(&headers)?,
    )
    .await?;
    state.publish(change);
    Ok((StatusCode::CREATED, Json(ticket)).into_response())
}

async fn update_ticket(
    State(state): State<AppState>,
    Path(ticket_id): Path<Uuid>,
    headers: HeaderMap,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let body: UpdateTicketBody = extract::parse(raw)?;
    let (ticket, change) = work::update_ticket(
        &state.pool,
        user_id,
        ticket_id,
        body.expected_revision,
        TicketUpdate {
            title: body.title,
            description: body.description,
            assignee_id: body.assignee_id,
            priority: body.priority,
            due_on: body.due_on,
            label_ids: body.label_ids,
            parent_id: body.parent_id,
            type_id: body.type_id,
            estimate: body.estimate.map(|e| e.map(Into::into)),
            schedule: body.schedule.map(|s| s.map(Into::into)),
        },
        &key(&headers)?,
    )
    .await?;
    state.publish(change);
    Ok(Json(ticket).into_response())
}

async fn get_ticket(
    State(state): State<AppState>,
    Path(ticket_id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    Ok(Json(work::get_ticket(&state.pool, user_id, ticket_id).await?).into_response())
}

async fn ticket_comments(
    State(state): State<AppState>,
    Path(ticket_id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    Ok(Json(work::comments(&state.pool, user_id, ticket_id).await?).into_response())
}

async fn create_ticket_comment(
    State(state): State<AppState>,
    Path(ticket_id): Path<Uuid>,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let body: CreateCommentBody = extract::parse(raw)?;
    let (comment, change) =
        work::create_comment(&state.pool, user_id, ticket_id, body.body, &body.file_ids).await?;
    state.publish([change]);
    Ok((StatusCode::CREATED, Json(comment)).into_response())
}

async fn edit_ticket_comment(
    State(state): State<AppState>,
    Path((ticket_id, comment_id)): Path<(Uuid, Uuid)>,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let body: EditCommentBody = extract::parse(raw)?;
    let (comment, change) =
        work::edit_comment(&state.pool, user_id, ticket_id, comment_id, body.body).await?;
    state.publish([change]);
    Ok(Json(comment).into_response())
}

async fn delete_ticket_comment(
    State(state): State<AppState>,
    Path((ticket_id, comment_id)): Path<(Uuid, Uuid)>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    let change = work::delete_comment(
        &state.pool,
        state.objects.as_ref(),
        user_id,
        ticket_id,
        comment_id,
    )
    .await?;
    state.publish([change]);
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn change_ticket_status(
    State(state): State<AppState>,
    Path(ticket_id): Path<Uuid>,
    headers: HeaderMap,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let body: ChangeStatusBody = extract::parse(raw)?;
    let (ticket, change) = work::change_status(
        &state.pool,
        user_id,
        ticket_id,
        body.expected_revision,
        body.status_id,
        &key(&headers)?,
    )
    .await?;
    state.publish(change);
    Ok(Json(ticket).into_response())
}

async fn ticket_revisions(
    State(state): State<AppState>,
    Path(ticket_id): Path<Uuid>,
    extract::Q(page): extract::Q<PageQuery>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    let before = page
        .before
        .as_deref()
        .map(|text| {
            text.parse::<i64>()
                .map_err(|_| ApiError::validation("invalid before"))
        })
        .transpose()?;
    Ok(
        Json(work::revisions(&state.pool, user_id, ticket_id, page.limit, before).await?)
            .into_response(),
    )
}

async fn ticket_revision_detail(
    State(state): State<AppState>,
    Path((ticket_id, revision_no)): Path<(Uuid, i64)>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    Ok(
        Json(work::revision_detail(&state.pool, user_id, ticket_id, revision_no).await?)
            .into_response(),
    )
}

async fn restore_ticket(
    State(state): State<AppState>,
    Path(ticket_id): Path<Uuid>,
    headers: HeaderMap,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let body: RestoreTicketBody = extract::parse(raw)?;
    let (ticket, change) = work::restore_ticket(
        &state.pool,
        user_id,
        ticket_id,
        body.expected_revision,
        work::RestoreInput {
            source_revision: body.source_revision,
            fields: body.fields,
        },
        &key(&headers)?,
    )
    .await?;
    state.publish(change);
    Ok(Json(ticket).into_response())
}

async fn ticket_children(
    State(state): State<AppState>,
    Path(ticket_id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    Ok(Json(work::children(&state.pool, user_id, ticket_id).await?).into_response())
}

async fn ticket_activity(
    State(state): State<AppState>,
    Path(ticket_id): Path<Uuid>,
    extract::Q(page): extract::Q<PageQuery>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    Ok(Json(
        work::ticket_activity(
            &state.pool,
            user_id,
            ticket_id,
            page.limit,
            page.before.as_deref(),
        )
        .await?,
    )
    .into_response())
}

async fn project_activity(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    Ok(Json(work::activity(&state.pool, user_id, project_id).await?).into_response())
}
