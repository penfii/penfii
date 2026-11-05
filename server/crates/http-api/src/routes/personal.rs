//! What a person keeps for themselves: favorites, Later, reminders and drafts.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use penfii_core::drafts;
use penfii_core::personal;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::AppState;
use crate::error::ApiError;
use crate::extract::{self, Actor, RawJson};

use axum::Router;
use axum::routing::{delete, get, patch, put};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/drafts", put(save_draft).get(load_draft))
        .route("/v1/drafts/{id}", delete(delete_draft))
        .route("/v1/workspaces/{id}/drafts", get(list_drafts))
        .route(
            "/v1/workspaces/{id}/favorites",
            get(list_favorites).put(add_favorite),
        )
        .route("/v1/workspaces/{id}/favorites/order", put(order_favorites))
        .route(
            "/v1/workspaces/{id}/favorites/{kind}/{target_id}",
            delete(remove_favorite),
        )
        .route("/v1/workspaces/{id}/later", get(list_later).post(add_later))
        .route("/v1/later/{id}", patch(update_later).delete(remove_later))
        .route(
            "/v1/workspaces/{id}/favorite-folders",
            get(list_favorite_folders).post(create_favorite_folder),
        )
        .route(
            "/v1/workspaces/{id}/favorite-folders/order",
            put(order_favorite_folders),
        )
        .route(
            "/v1/favorite-folders/{id}",
            patch(rename_favorite_folder).delete(delete_favorite_folder),
        )
        .route(
            "/v1/workspaces/{id}/favorites/{kind}/{target_id}/folder",
            put(move_favorite),
        )
        .route(
            "/v1/later/{id}/reminder",
            put(set_reminder).delete(cancel_reminder),
        )
}

// ---- Personal Work: Favorites, Later, Pins ------------------------------------------------

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SavedTargetBody {
    kind: String,
    target_id: Uuid,
    #[serde(default)]
    note: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FavoriteOrderBody {
    items: Vec<FavoriteRef>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FavoriteRef {
    kind: String,
    target_id: Uuid,
}

async fn list_favorites(
    State(state): State<AppState>,
    Path(workspace_id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    Ok(Json(personal::list_favorites(&state.pool, user_id, workspace_id).await?).into_response())
}

async fn add_favorite(
    State(state): State<AppState>,
    Path(workspace_id): Path<Uuid>,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let body: SavedTargetBody = extract::parse(raw)?;
    let item = personal::add_favorite(
        &state.pool,
        user_id,
        workspace_id,
        &body.kind,
        body.target_id,
    )
    .await?;
    Ok(Json(item).into_response())
}

async fn remove_favorite(
    State(state): State<AppState>,
    Path((workspace_id, kind, target_id)): Path<(Uuid, String, Uuid)>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    personal::remove_favorite(&state.pool, user_id, workspace_id, &kind, target_id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn order_favorites(
    State(state): State<AppState>,
    Path(workspace_id): Path<Uuid>,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let body: FavoriteOrderBody = extract::parse(raw)?;
    personal::order_favorites(
        &state.pool,
        user_id,
        workspace_id,
        body.items
            .into_iter()
            .map(|i| (i.kind, i.target_id))
            .collect(),
    )
    .await?;
    Ok(Json(personal::list_favorites(&state.pool, user_id, workspace_id).await?).into_response())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct LaterQuery {
    #[serde(default)]
    state: Option<String>,
}

async fn list_later(
    State(state): State<AppState>,
    Path(workspace_id): Path<Uuid>,
    extract::Q(query): extract::Q<LaterQuery>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    let state_name = query.state.as_deref().unwrap_or("ACTIVE");
    Ok(
        Json(personal::list_later(&state.pool, user_id, workspace_id, state_name).await?)
            .into_response(),
    )
}

async fn add_later(
    State(state): State<AppState>,
    Path(workspace_id): Path<Uuid>,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let body: SavedTargetBody = extract::parse(raw)?;
    let item = personal::add_later(
        &state.pool,
        user_id,
        workspace_id,
        &body.kind,
        body.target_id,
        body.note,
    )
    .await?;
    Ok(Json(item).into_response())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateLaterBody {
    expected_version: i64,
    state: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    note: Option<Option<String>>,
}

fn double_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    Ok(Some(<Option<T> as serde::Deserialize>::deserialize(
        deserializer,
    )?))
}

async fn update_later(
    State(state): State<AppState>,
    Path(later_id): Path<Uuid>,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let body: UpdateLaterBody = extract::parse(raw)?;
    let item = personal::update_later(
        &state.pool,
        user_id,
        later_id,
        body.expected_version,
        body.state,
        body.note,
    )
    .await?;
    Ok(Json(item).into_response())
}

async fn remove_later(
    State(state): State<AppState>,
    Path(later_id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    personal::remove_later(&state.pool, user_id, later_id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

// ---- Drafts ---------------------------------------------------------------------------------

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SaveDraftBody {
    context_kind: String,
    context_id: Uuid,
    #[serde(default)]
    #[allow(dead_code)]
    generation: Option<Uuid>,
    #[serde(default)]
    expected_version: Option<i64>,
    payload: SaveDraftPayload,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveDraftPayload {
    schema_version: i16,
    body: Value,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DraftContextQuery {
    context_kind: String,
    context_id: Uuid,
}

async fn save_draft(
    State(state): State<AppState>,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let body: SaveDraftBody = extract::parse(raw)?;
    if body.payload.schema_version != 1 {
        return Err(ApiError::validation("unsupported draft schema version"));
    }
    let (draft, created) = drafts::save_draft(
        &state.pool,
        user_id,
        &body.context_kind,
        body.context_id,
        body.expected_version,
        body.payload.body,
    )
    .await?;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(draft)).into_response())
}

async fn load_draft(
    State(state): State<AppState>,
    extract::Q(query): extract::Q<DraftContextQuery>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    Ok(
        match drafts::load_draft(&state.pool, user_id, &query.context_kind, query.context_id)
            .await?
        {
            Some(draft) => Json(draft).into_response(),
            None => StatusCode::NO_CONTENT.into_response(),
        },
    )
}

async fn delete_draft(
    State(state): State<AppState>,
    Path(draft_id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    drafts::delete_draft(&state.pool, user_id, draft_id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn list_drafts(
    State(state): State<AppState>,
    Path(workspace_id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    Ok(Json(drafts::list_drafts(&state.pool, user_id, workspace_id).await?).into_response())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReminderBody {
    due_at: chrono::DateTime<chrono::Utc>,
    time_zone: String,
}

async fn set_reminder(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let body: ReminderBody = extract::parse(raw)?;
    let reminder =
        penfii_core::reminders::set(&state.pool, user_id, id, body.due_at, &body.time_zone).await?;
    Ok(Json(reminder).into_response())
}

async fn cancel_reminder(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    penfii_core::reminders::cancel(&state.pool, user_id, id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FolderBody {
    name: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FolderOrderBody {
    ids: Vec<Uuid>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MoveFavoriteBody {
    folder_id: Option<Uuid>,
}

async fn list_favorite_folders(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    Ok(
        Json(json!({"items": personal::list_folders(&state.pool, user_id, id).await?}))
            .into_response(),
    )
}

async fn create_favorite_folder(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let body: FolderBody = extract::parse(raw)?;
    let folder = personal::create_folder(&state.pool, user_id, id, &body.name).await?;
    Ok((StatusCode::CREATED, Json(folder)).into_response())
}

async fn order_favorite_folders(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let body: FolderOrderBody = extract::parse(raw)?;
    personal::order_folders(&state.pool, user_id, id, body.ids).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn rename_favorite_folder(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let body: FolderBody = extract::parse(raw)?;
    Ok(Json(personal::rename_folder(&state.pool, user_id, id, &body.name).await?).into_response())
}

async fn delete_favorite_folder(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    let moved = personal::delete_folder(&state.pool, user_id, id).await?;
    Ok(Json(json!({"movedToTopLevel": moved})).into_response())
}

async fn move_favorite(
    State(state): State<AppState>,
    Path((id, kind, target)): Path<(Uuid, String, Uuid)>,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let body: MoveFavoriteBody = extract::parse(raw)?;
    personal::move_favorite(&state.pool, user_id, id, &kind, target, body.folder_id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}
