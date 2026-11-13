//! Channels, direct messages, messages, reactions and read state.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use penfii_contracts::{
    CreateChannelRequest, CreateDmRequest, EditMessageRequest, ManualUnreadRequest,
    ReactionRequest, SendMessageRequest, SeqLimitQuery, SetChannelMemberRoleRequest,
    UpdateChannelRequest, UpdateReadRequest,
};
use penfii_core::conversation;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{finish, key};
use crate::AppState;
use crate::error::ApiError;
use crate::extract::{self, Actor, Body, Q, RawJson};

use axum::Router;
use axum::routing::{get, patch, post, put};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/workspaces/{id}/channels", post(create_channel))
        .route("/v1/channels/{id}", patch(update_channel))
        .route("/v1/channels/{id}/join", post(join_channel))
        .route("/v1/channels/{id}/members", get(list_channel_members))
        .route(
            "/v1/channels/{id}/members/{user_id}",
            put(add_channel_member)
                .patch(set_channel_member_role)
                .delete(leave_channel),
        )
        .route("/v1/workspaces/{id}/dms", post(create_dm))
        .route("/v1/conversations/{id}", get(get_conversation))
        .route(
            "/v1/conversations/{id}/messages",
            get(list_messages).post(send_message),
        )
        .route("/v1/messages/{id}/thread", get(list_thread))
        .route(
            "/v1/messages/{id}",
            patch(edit_message).delete(delete_message),
        )
        .route("/v1/messages/{id}/history", get(message_history))
        .route(
            "/v1/messages/{id}/reactions",
            put(add_reaction).delete(remove_reaction),
        )
        .route(
            "/v1/conversations/{id}/read",
            get(get_read).post(update_read),
        )
        .route("/v1/conversations/{id}/manual-unread", post(manual_unread))
        .route(
            "/v1/threads/{rootId}/read",
            get(get_thread_read).post(update_thread_read),
        )
        .route(
            "/v1/threads/{rootId}/manual-unread",
            post(manual_thread_unread),
        )
        .route("/v1/conversations/{id}/changes", get(list_changes))
}

async fn create_channel(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
    Body(body): Body<CreateChannelRequest>,
) -> Result<Response, ApiError> {
    let channel = conversation::create_channel(
        &state.pool,
        user_id,
        id,
        &body.name,
        body.description.as_deref(),
        &body.visibility,
        body.team_id,
        body.project_id,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(channel)).into_response())
}

async fn update_channel(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
    Body(body): Body<UpdateChannelRequest>,
) -> Result<Response, ApiError> {
    let (channel, notices) = conversation::update_channel(
        &state.pool,
        user_id,
        id,
        body.visibility.as_deref(),
        body.description.as_deref(),
        body.allow_member_invites,
    )
    .await?;
    state.publish(notices);
    Ok(Json(channel).into_response())
}

async fn join_channel(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    Ok(Json(conversation::join_channel(&state.pool, user_id, id).await?).into_response())
}

async fn list_channel_members(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    let items = conversation::list_channel_members(&state.pool, user_id, id).await?;
    Ok(Json(json!({ "items": items })).into_response())
}

async fn add_channel_member(
    State(state): State<AppState>,
    Path((id, member_id)): Path<(Uuid, Uuid)>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    let (member, change) =
        conversation::add_channel_member(&state.pool, user_id, id, member_id).await?;
    state.publish(change);
    Ok(Json(member).into_response())
}

async fn set_channel_member_role(
    State(state): State<AppState>,
    Path((id, member_id)): Path<(Uuid, Uuid)>,
    Actor(user_id): Actor,
    Body(body): Body<SetChannelMemberRoleRequest>,
) -> Result<Response, ApiError> {
    let (member, change) =
        conversation::set_channel_member_role(&state.pool, user_id, id, member_id, &body.role)
            .await?;
    state.publish(change);
    Ok(Json(member).into_response())
}

async fn leave_channel(
    State(state): State<AppState>,
    Path((id, member_id)): Path<(Uuid, Uuid)>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    let changes = conversation::leave_channel(&state.pool, user_id, id, member_id).await?;
    state.publish(changes);
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn create_dm(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
    Body(body): Body<CreateDmRequest>,
) -> Result<Response, ApiError> {
    let dm = conversation::create_dm(&state.pool, user_id, id, &body.user_ids).await?;
    Ok((StatusCode::CREATED, Json(dm)).into_response())
}

async fn get_conversation(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    Ok(Json(conversation::get_conversation(&state.pool, user_id, id).await?).into_response())
}

async fn list_messages(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Q(query): Q<SeqLimitQuery>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    let messages =
        conversation::list_messages(&state.pool, user_id, id, query.before_seq, query.limit)
            .await?;
    Ok(Json(messages).into_response())
}

async fn send_message(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let idempotency_key = key(&headers)?;
    let body: SendMessageRequest = extract::parse(raw.clone())?;
    let outcome = conversation::send_message(
        &state.pool,
        &state.limits,
        conversation::SendMessage {
            actor_id: user_id,
            conversation_id: id,
            idempotency_key,
            body_schema_version: body.body_schema_version,
            body: body.body,
            thread_root_id: body.thread_root_id,
            reply_to_id: body.reply_to_id,
            file_ids: body.file_ids,
            raw,
        },
    )
    .await?;
    finish(&state, outcome)
}

async fn list_thread(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Q(query): Q<SeqLimitQuery>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    let messages =
        conversation::list_thread(&state.pool, user_id, id, query.after_seq, query.limit).await?;
    Ok(Json(messages).into_response())
}

async fn edit_message(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    let idempotency_key = key(&headers)?;
    let body: EditMessageRequest = extract::parse(raw.clone())?;
    let outcome = conversation::edit_message(
        &state.pool,
        &state.limits,
        conversation::EditMessage {
            actor_id: user_id,
            message_id: id,
            idempotency_key,
            expected_version: body.expected_version,
            body: body.body,
            raw,
        },
    )
    .await?;
    finish(&state, outcome)
}

async fn delete_message(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    let idempotency_key = key(&headers)?;
    let raw = json!({});
    let outcome = conversation::delete_message(
        &state.pool,
        conversation::DeleteMessage {
            actor_id: user_id,
            message_id: id,
            idempotency_key,
            raw,
        },
    )
    .await?;
    finish(&state, outcome)
}

async fn message_history(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    Ok(Json(conversation::message_history(&state.pool, user_id, id).await?).into_response())
}

async fn add_reaction(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    reaction(&state, user_id, id, &headers, raw, true).await
}

async fn remove_reaction(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    reaction(&state, user_id, id, &headers, raw, false).await
}

async fn reaction(
    state: &AppState,
    user_id: Uuid,
    message_id: Uuid,
    headers: &HeaderMap,
    raw: Value,
    add: bool,
) -> Result<Response, ApiError> {
    let idempotency_key = key(headers)?;
    let body: ReactionRequest = extract::parse(raw.clone())?;
    let cmd = conversation::ReactionCommand {
        actor_id: user_id,
        message_id,
        idempotency_key,
        emoji: body.emoji,
        raw,
    };
    let outcome = if add {
        conversation::add_reaction(&state.pool, cmd).await?
    } else {
        conversation::remove_reaction(&state.pool, cmd).await?
    };
    finish(state, outcome)
}

async fn get_read(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    Ok(Json(conversation::get_conversation_read(&state.pool, user_id, id).await?).into_response())
}

async fn update_read(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    read_update(&state, user_id, id, &headers, raw, false).await
}

async fn get_thread_read(
    State(state): State<AppState>,
    Path(root_id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    Ok(Json(conversation::get_thread_read(&state.pool, user_id, root_id).await?).into_response())
}

async fn update_thread_read(
    State(state): State<AppState>,
    Path(root_id): Path<Uuid>,
    headers: HeaderMap,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    read_update(&state, user_id, root_id, &headers, raw, true).await
}

async fn read_update(
    state: &AppState,
    user_id: Uuid,
    target_id: Uuid,
    headers: &HeaderMap,
    raw: Value,
    thread: bool,
) -> Result<Response, ApiError> {
    let idempotency_key = key(headers)?;
    let body: UpdateReadRequest = extract::parse(raw.clone())?;
    let cmd = conversation::UpdateRead {
        actor_id: user_id,
        target_id,
        idempotency_key,
        generation: body.generation,
        read_through_seq: body.read_through_seq,
        scope: body.scope,
        observed: body.observed,
        raw,
    };
    let outcome = if thread {
        conversation::update_thread_read(&state.pool, cmd).await?
    } else {
        conversation::update_conversation_read(&state.pool, cmd).await?
    };
    finish(state, outcome)
}

async fn manual_unread(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    manual(&state, user_id, id, &headers, raw, false).await
}

async fn manual_thread_unread(
    State(state): State<AppState>,
    Path(root_id): Path<Uuid>,
    headers: HeaderMap,
    Actor(user_id): Actor,
    RawJson(raw): RawJson,
) -> Result<Response, ApiError> {
    manual(&state, user_id, root_id, &headers, raw, true).await
}

async fn manual(
    state: &AppState,
    user_id: Uuid,
    target_id: Uuid,
    headers: &HeaderMap,
    raw: Value,
    thread: bool,
) -> Result<Response, ApiError> {
    let idempotency_key = key(headers)?;
    let body: ManualUnreadRequest = extract::parse(raw.clone())?;
    let cmd = conversation::ManualUnread {
        actor_id: user_id,
        target_id,
        idempotency_key,
        from_seq: body.from_seq,
        raw,
    };
    let outcome = if thread {
        conversation::manual_thread_unread(&state.pool, cmd).await?
    } else {
        conversation::manual_conversation_unread(&state.pool, cmd).await?
    };
    finish(state, outcome)
}

async fn list_changes(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Q(query): Q<SeqLimitQuery>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    let changes =
        conversation::list_changes(&state.pool, user_id, id, query.after_seq, query.limit).await?;
    Ok(Json(changes).into_response())
}
