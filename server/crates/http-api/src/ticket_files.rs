//! Files on a Ticket's body. Comment files travel with the comment itself
//! (`POST /v1/tickets/{id}/comments` takes `fileIds`) and download through
//! `/v1/attachments/{id}/content` like message attachments.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use penfii_core::ticket_files;
use serde::Deserialize;
use uuid::Uuid;

use crate::AppState;
use crate::error::ApiError;
use crate::extract::{self, Actor, RawJson};

use axum::Router;
use axum::routing::{delete, get};

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/v1/tickets/{id}/attachments",
            get(list_ticket_attachments).post(attach_ticket_file),
        )
        .route(
            "/v1/tickets/{id}/attachments/{attachmentId}",
            delete(detach_ticket_file),
        )
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AttachBody {
    file_id: Uuid,
}

pub async fn list_ticket_attachments(
    State(state): State<AppState>,
    Path(ticket_id): Path<Uuid>,
    Actor(user): Actor,
) -> Result<Response, ApiError> {
    Ok(Json(ticket_files::list_body(&state.pool, user, ticket_id).await?).into_response())
}

pub async fn attach_ticket_file(
    State(state): State<AppState>,
    Path(ticket_id): Path<Uuid>,
    Actor(user): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let body: AttachBody = extract::parse(raw)?;
    let (attached, change) =
        ticket_files::attach_body(&state.pool, user, ticket_id, body.file_id).await?;
    state.publish(change);
    Ok((StatusCode::CREATED, Json(attached)).into_response())
}

pub async fn detach_ticket_file(
    State(state): State<AppState>,
    Path((ticket_id, attachment_id)): Path<(Uuid, Uuid)>,
    Actor(user): Actor,
) -> Result<Response, ApiError> {
    let change = ticket_files::detach_body(
        &state.pool,
        state.objects.as_ref(),
        user,
        ticket_id,
        attachment_id,
    )
    .await?;
    state.publish([change]);
    Ok(StatusCode::NO_CONTENT.into_response())
}
