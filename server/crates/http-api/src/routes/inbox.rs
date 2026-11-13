//! The Inbox, subscriptions and notification preferences.

use axum::Json;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use penfii_core::CoreError;
use serde_json::json;
use uuid::Uuid;

use crate::AppState;
use crate::error::ApiError;
use crate::extract::{Actor, Body, Q};

use axum::Router;
use axum::routing::{get, post, put};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/workspaces/{id}/inbox", get(inbox_list))
        .route(
            "/v1/workspaces/{id}/inbox/commands/read-all",
            post(inbox_read_all),
        )
        .route("/v1/inbox/{id}/commands/{command}", post(inbox_command))
        .route(
            "/v1/tickets/{id}/subscription",
            get(ticket_subscription)
                .put(watch_ticket)
                .delete(unwatch_ticket),
        )
        .route(
            "/v1/projects/{id}/subscription",
            get(project_subscription)
                .put(watch_project)
                .delete(unwatch_project),
        )
        .route(
            "/v1/messages/{id}/subscription",
            get(thread_subscription)
                .put(watch_thread)
                .delete(unwatch_thread),
        )
        .route("/v1/workspaces/{id}/subscriptions", get(list_subscriptions))
        .route(
            "/v1/me/notification-preferences",
            get(get_preferences).put(put_preferences),
        )
        .route("/v1/me/muted-channels", get(muted_channels))
        .route(
            "/v1/channels/{id}/mute",
            put(mute_channel).delete(unmute_channel),
        )
}

// ---- Inbox, Watch, notification preferences ----------------------------------------------------

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct InboxQuery {
    #[serde(default)]
    filter: Option<String>,
    #[serde(default)]
    state: Option<String>,
    /// `inbox` (default) or `archived`.
    #[serde(default)]
    view: Option<String>,
    #[serde(default)]
    include_snoozed: Option<bool>,
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    limit: Option<i64>,
}

async fn inbox_list(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Q(query): Q<InboxQuery>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    let page = penfii_core::inbox::list(
        &state.pool,
        user_id,
        id,
        penfii_core::inbox::ListOptions {
            view: penfii_core::inbox::View::parse(query.view.as_deref())?,
            filter: query.filter.as_deref().unwrap_or("all"),
            unread_only: query.state.as_deref() == Some("unread"),
            include_snoozed: query.include_snoozed.unwrap_or(false),
        },
        query.cursor.as_deref(),
        query.limit,
    )
    .await?;
    Ok(Json(page).into_response())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct InboxCommandBody {
    expected_version: i64,
    #[serde(default)]
    until: Option<chrono::DateTime<chrono::Utc>>,
}

async fn inbox_command(
    State(state): State<AppState>,
    Path((id, command)): Path<(Uuid, String)>,
    Actor(user_id): Actor,
    Body(body): Body<InboxCommandBody>,
) -> Result<Response, ApiError> {
    use penfii_core::inbox::Command;
    let command = match command.as_str() {
        "read" => Command::Read,
        "unread" => Command::Unread,
        "archive" => Command::Archive,
        "unarchive" => Command::Unarchive,
        "snooze" => Command::Snooze(
            body.until
                .ok_or_else(|| ApiError::validation("snooze needs `until`"))?,
        ),
        _ => return Err(ApiError::from_core(CoreError::NotFound)),
    };
    let value =
        penfii_core::inbox::command(&state.pool, user_id, id, body.expected_version, command)
            .await?;
    state.publish([penfii_core::ChangeNotice::Inbox { user_id }]);
    Ok(Json(value).into_response())
}

#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ReadAllBody {
    #[serde(default)]
    filter: Option<String>,
}

async fn inbox_read_all(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
    Body(body): Body<ReadAllBody>,
) -> Result<Response, ApiError> {
    let changed = penfii_core::inbox::read_all(
        &state.pool,
        user_id,
        id,
        body.filter.as_deref().unwrap_or("all"),
    )
    .await?;
    Ok(Json(json!({"changed": changed})).into_response())
}

async fn subscription_get(
    state: &AppState,
    user_id: Uuid,
    kind: &str,
    id: Uuid,
) -> Result<Response, ApiError> {
    Ok(Json(penfii_core::watch::status(&state.pool, user_id, kind, id).await?).into_response())
}

async fn subscription_set(
    state: &AppState,
    user_id: Uuid,
    kind: &str,
    id: Uuid,
    on: bool,
) -> Result<Response, ApiError> {
    Ok(Json(penfii_core::watch::set(&state.pool, user_id, kind, id, on).await?).into_response())
}

async fn ticket_subscription(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    subscription_get(&s, user_id, "TICKET", id).await
}

async fn watch_ticket(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    subscription_set(&s, user_id, "TICKET", id, true).await
}

async fn unwatch_ticket(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    subscription_set(&s, user_id, "TICKET", id, false).await
}

async fn project_subscription(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    subscription_get(&s, user_id, "PROJECT", id).await
}

async fn watch_project(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    subscription_set(&s, user_id, "PROJECT", id, true).await
}

async fn unwatch_project(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    subscription_set(&s, user_id, "PROJECT", id, false).await
}

async fn thread_subscription(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    subscription_get(&s, user_id, "THREAD", id).await
}

async fn watch_thread(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    subscription_set(&s, user_id, "THREAD", id, true).await
}

async fn unwatch_thread(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    subscription_set(&s, user_id, "THREAD", id, false).await
}

#[derive(serde::Deserialize)]
struct SubscriptionsQuery {
    #[serde(default)]
    kind: Option<String>,
}

async fn list_subscriptions(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Q(query): Q<SubscriptionsQuery>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    let items = penfii_core::watch::list(&state.pool, user_id, id, query.kind.as_deref()).await?;
    Ok(Json(json!({"items": items})).into_response())
}

async fn get_preferences(
    State(state): State<AppState>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    Ok(Json(penfii_core::preferences::view(&state.pool, user_id).await?).into_response())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PreferencesBody {
    #[serde(flatten)]
    preferences: penfii_core::preferences::Preferences,
    expected_version: i64,
}

async fn put_preferences(
    State(state): State<AppState>,
    Actor(user_id): Actor,
    Body(body): Body<PreferencesBody>,
) -> Result<Response, ApiError> {
    let saved = penfii_core::preferences::save(
        &state.pool,
        user_id,
        body.preferences,
        body.expected_version,
    )
    .await?;
    Ok(Json(saved).into_response())
}

async fn muted_channels(
    State(state): State<AppState>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    Ok(Json(json!({"channelIds": penfii_core::preferences::muted_channels(&state.pool, user_id).await?})).into_response())
}

async fn mute_channel(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    penfii_core::preferences::set_mute(&state.pool, user_id, id, true).await?;
    Ok(Json(json!({"channelId": id, "muted": true})).into_response())
}

async fn unmute_channel(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    penfii_core::preferences::set_mute(&state.pool, user_id, id, false).await?;
    Ok(Json(json!({"channelId": id, "muted": false})).into_response())
}
