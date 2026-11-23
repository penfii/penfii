//! Live conversation changes. Read cursors are not published on this stream.

use std::time::Duration;

use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use penfii_core::docs::collab::close;
use penfii_core::platform::ChangeNotice;
use penfii_core::{CoreError, conversation, identity, work};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::broadcast::error::RecvError;
use uuid::Uuid;

use crate::AppState;
use crate::error::ApiError;
use crate::extract::{self, Actor};
use crate::origin::AllowedOrigin;

use axum::Router;
use axum::routing::get;

// How often an open socket asks again whether the session it was opened with is still alive
// (`AppState::ws_session_recheck`, 15 seconds). A web session that was revoked (a device signed out, a
// password changed or reset, the account disabled) or has expired ends its sockets within this time;
// before this check a socket stayed open for as long as it was connected.

/// The session a socket was opened with, asked about again on a timer. Only a session that is
/// really over ends the socket; a database that does not answer for a moment does not (the next
/// check asks again).
struct SessionWatch {
    pool: sqlx::PgPool,
    token: String,
    every: tokio::time::Interval,
}

impl SessionWatch {
    async fn new(state: &AppState, headers: &HeaderMap) -> Self {
        let mut every =
            tokio::time::interval(state.ws_session_recheck.max(Duration::from_millis(10)));
        every.tick().await;
        Self {
            pool: state.pool.clone(),
            token: extract::session_token(headers).unwrap_or_default(),
            every,
        }
    }

    /// Resolves when a check is due. Cancelling it loses nothing, so it can be a `select!` arm; the check
    /// itself is [`Self::ended`], run in the arm's body so a busy socket cannot keep cancelling it.
    async fn due(&mut self) {
        self.every.tick().await;
    }

    /// Whether the session is over.
    async fn ended(&self) -> bool {
        matches!(
            identity::authenticate(&self.pool, &self.token).await,
            Err(CoreError::Unauthenticated)
        )
    }
}

/// Tells the browser why the socket ends: the session is over (the same code as the co-editing socket).
async fn close_session<S>(sink: &mut S)
where
    S: SinkExt<Message> + Unpin,
{
    let _ = sink
        .send(Message::Close(Some(CloseFrame {
            code: close::SESSION,
            reason: "session ended".into(),
        })))
        .await;
}

/// The most changes a reconnecting Project socket is caught up with; further behind than this it reads the Project again.
const PROJECT_REPLAY_LIMIT: i64 = 500;

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/ws", get(ws))
        .route("/v1/ws/user", get(user_ws))
        .route("/v1/ws/projects/{id}", get(project_ws))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WsQuery {
    conversation_id: Uuid,
}

#[derive(Deserialize)]
struct Hello {
    v: i32,
    #[serde(rename = "type")]
    kind: String,
    #[serde(rename = "afterChangeSeq")]
    after_change_seq: i64,
}

/// What a client may send on an event socket: a hello naming what it wants (a conversation id, a change
/// number) and pings. A few hundred bytes; the library default is 64 MiB per message.
const EVENT_SOCKET_MAX_MESSAGE: usize = 8 * 1024;

pub(crate) async fn ws(
    State(state): State<AppState>,
    Query(query): Query<WsQuery>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
    _: AllowedOrigin,
    Actor(user_id): Actor,
) -> Result<axum::response::Response, ApiError> {
    conversation::readable_head(&state.pool, user_id, query.conversation_id).await?;
    let watch = SessionWatch::new(&state, &headers).await;
    Ok(upgrade
        .max_message_size(EVENT_SOCKET_MAX_MESSAGE)
        .max_frame_size(EVENT_SOCKET_MAX_MESSAGE)
        .on_upgrade(move |socket| handle(socket, state, watch, user_id, query.conversation_id))
        .into_response())
}

pub(crate) async fn project_ws(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
    _: AllowedOrigin,
    Actor(user_id): Actor,
) -> Result<axum::response::Response, ApiError> {
    work::board(&state.pool, project_id, user_id).await?;
    let watch = SessionWatch::new(&state, &headers).await;
    Ok(upgrade
        .max_message_size(EVENT_SOCKET_MAX_MESSAGE)
        .max_frame_size(EVENT_SOCKET_MAX_MESSAGE)
        .on_upgrade(move |socket| handle_project(socket, state, watch, user_id, project_id))
        .into_response())
}

async fn handle_project(
    socket: WebSocket,
    state: AppState,
    mut watch: SessionWatch,
    user_id: Uuid,
    project_id: Uuid,
) {
    let (mut sink, mut stream) = socket.split();
    let mut rx = state.changes.subscribe();
    let hello = match tokio::time::timeout(Duration::from_secs(30), next_text(&mut stream)).await {
        Ok(Some(text)) => text,
        _ => return,
    };
    let Ok(hello) = serde_json::from_str::<Hello>(&hello) else {
        return;
    };
    if hello.v != 1 || hello.kind != "hello" {
        return;
    }
    let Ok(_board) = work::board(&state.pool, project_id, user_id).await else {
        return;
    };
    // The socket is already subscribed, so a change committed from here on reaches `rx`. The feed is read after that:
    // everything up to `head` comes from it (or is already in the client's view), everything after from `rx`.
    let Ok(feed) = work::project_changes_since(
        &state.pool,
        project_id,
        hello.after_change_seq.max(0),
        PROJECT_REPLAY_LIMIT,
    )
    .await
    else {
        return;
    };
    let head = feed.head;
    let after = hello.after_change_seq;
    // `after` 0 is a client with nothing yet (it reads the Board when it gets `project.ready`). Otherwise it holds
    // everything up to `after`: it is caught up from the feed when the feed can say what happened, and told to read
    // the Project again when it cannot (a position the feed does not know, or too far behind).
    if after > 0 {
        if after > head || after < feed.floor || feed.truncated {
            if send_json(
                &mut sink,
                &json!({"v":1,"type":"project.resync_required","projectId":project_id}),
            )
            .await
            .is_err()
            {
                return;
            }
        } else {
            let mut visible = std::collections::HashMap::new();
            for change in &feed.changes {
                let seen = match visible.get(&change.ticket_id) {
                    Some(seen) => *seen,
                    None => {
                        // The same check as for a live change: Project role and the Ticket's channel.
                        match work::ticket_change_visible(
                            &state.pool,
                            project_id,
                            change.ticket_id,
                            user_id,
                        )
                        .await
                        {
                            Ok(seen) => {
                                visible.insert(change.ticket_id, seen);
                                seen
                            }
                            Err(_) => return,
                        }
                    }
                };
                if !seen {
                    continue;
                }
                let frame = json!({"v":1,"type":"project.ticket.change","projectId":project_id,"ticketId":change.ticket_id,"ticketRevisionNo":change.ticket_revision_no,"changeKind":change.change_kind,"changeSeq":change.change_seq});
                if send_json(&mut sink, &frame).await.is_err() {
                    return;
                }
            }
        }
    }
    let _ = send_json(
        &mut sink,
        &json!({"v":1,"type":"project.ready","projectId":project_id,"headSeq":head}),
    )
    .await;
    loop {
        tokio::select! {
            incoming=stream.next()=>match incoming {Some(Ok(Message::Ping(payload)))=>{if sink.send(Message::Pong(payload)).await.is_err(){return;}},Some(Ok(Message::Close(_)))|None=>return,Some(Err(_))=>return,_=>{}},
            _=watch.due()=>{ if watch.ended().await { close_session(&mut sink).await; return; } },
            event=rx.recv()=>{
                let event = match event {
                    Ok(event) => event,
                    Err(RecvError::Lagged(_)) => {
                        let _ = send_json(&mut sink, &json!({"v":1,"type":"project.resync_required","projectId":project_id})).await;
                        return;
                    }
                    Err(RecvError::Closed) => return,
                };
                if matches!(event, ChangeNotice::Access { user_id: target } if target == user_id) {
                    return;
                }
                match event {
                    ChangeNotice::Ticket { project_id: event_project, change_seq, ticket_id, ticket_revision_no, change_kind } => {
                        if event_project != project_id { continue; }
                        // Changes up to `head` were sent from the feed above (or the client already holds them).
                        if change_seq <= head { continue; }
                        // One ticket, one small check (Project role and the ticket's channel): never the whole Board.
                        match work::ticket_change_visible(&state.pool, project_id, ticket_id, user_id).await {
                            Ok(true) => {}
                            Ok(false) => continue,
                            Err(_) => return,
                        }
                        let frame = json!({"v":1,"type":"project.ticket.change","projectId":project_id,"ticketId":ticket_id,"ticketRevisionNo":ticket_revision_no,"changeKind":change_kind,"changeSeq":change_seq});
                        if send_json(&mut sink, &frame).await.is_err() { return; }
                    }
                    // Many tickets changed at once: the client reads the Board again.
                    ChangeNotice::Project { project_id: event_project } => {
                        if event_project != project_id { continue; }
                        if work::project_readable(&state.pool, project_id, user_id).await.is_err() { return; }
                        let frame = json!({"v":1,"type":"project.resync_required","projectId":project_id});
                        if send_json(&mut sink, &frame).await.is_err() { return; }
                    }
                    _ => continue,
                }
            }
        }
    }
}

async fn handle(
    socket: WebSocket,
    state: AppState,
    mut watch: SessionWatch,
    user_id: Uuid,
    conversation_id: Uuid,
) {
    let (mut sink, mut stream) = socket.split();
    let mut rx = state.changes.subscribe();
    let hello = match tokio::time::timeout(Duration::from_secs(30), next_text(&mut stream)).await {
        Ok(Some(text)) => text,
        _ => return,
    };
    let Ok(hello) = serde_json::from_str::<Hello>(&hello) else {
        return;
    };
    if hello.v != 1 || hello.kind != "hello" {
        return;
    }
    let Ok(head) = conversation::readable_head(&state.pool, user_id, conversation_id).await else {
        return;
    };
    if hello.after_change_seq < head.replay_floor_seq {
        let _ = send_json(&mut sink, &json!({"v": 1, "type": "resync_required"})).await;
        return;
    }
    let Ok(mut after) = replay(
        &mut sink,
        &state,
        user_id,
        conversation_id,
        hello.after_change_seq,
    )
    .await
    else {
        return;
    };
    loop {
        tokio::select! {
            incoming = stream.next() => {
                match incoming {
                    Some(Ok(Message::Ping(payload))) => {
                        if sink.send(Message::Pong(payload)).await.is_err() {
                            return;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => return,
                    Some(Err(_)) => return,
                    _ => {}
                }
            }
            _ = watch.due() => {
                if watch.ended().await {
                    close_session(&mut sink).await;
                    return;
                }
            }
            event = rx.recv() => {
                let event = match event {
                    Ok(event) => event,
                    Err(RecvError::Lagged(_)) => {
                        let _ = send_json(&mut sink, &json!({"v": 1, "type": "resync_required"})).await;
                        return;
                    }
                    Err(RecvError::Closed) => return,
                };
                if matches!(event, ChangeNotice::Access { user_id: target } if target == user_id) {
                    return;
                }
                let ChangeNotice::Conversation { conversation_id: event_conversation, change_seq, message_id, message_version, change_kind } = event else {
                    continue;
                };
                if event_conversation != conversation_id || change_seq <= after {
                    continue;
                }
                if conversation::readable_head(&state.pool, user_id, conversation_id).await.is_err() {
                    return;
                }
                // Broadcast order is not commit order. A higher seq must not hide an earlier one:
                // the log is the order, and a hole that the log cannot fill asks the client to catch up.
                if change_seq > after + 1 {
                    after = match replay(&mut sink, &state, user_id, conversation_id, after).await {
                        Ok(after) => after,
                        Err(()) => return,
                    };
                    if change_seq <= after {
                        continue;
                    }
                    let _ = send_json(&mut sink, &json!({"v": 1, "type": "resync_required"})).await;
                    return;
                }
                let frame = json!({
                    "v": 1,
                    "type": "conversation.change",
                    "conversationId": event_conversation,
                    "changeSeq": change_seq,
                    "messageId": message_id,
                    "messageVersion": message_version,
                    "changeKind": change_kind,
                });
                if send_json(&mut sink, &frame).await.is_err() {
                    return;
                }
                after = change_seq;
            }
        }
    }
}

/// Sends every stored change after `after`, in commit order, and returns the cursor that was delivered.
/// A failed send does not advance past the change that did not go out.
async fn replay<S>(
    sink: &mut S,
    state: &AppState,
    user_id: Uuid,
    conversation_id: Uuid,
    mut after: i64,
) -> Result<i64, ()>
where
    S: SinkExt<Message> + Unpin,
{
    loop {
        let page = conversation::list_changes(
            &state.pool,
            user_id,
            conversation_id,
            Some(after),
            Some(100),
        )
        .await
        .map_err(|_| ())?;
        if page.is_empty() {
            break;
        }
        for change in &page {
            send_json(sink, &change_frame(change)).await?;
            after = change.change_seq;
        }
        if page.len() < 100 {
            break;
        }
    }
    Ok(after)
}

fn change_frame(change: &penfii_contracts::ConversationChange) -> Value {
    json!({
        "v": 1,
        "type": "conversation.change",
        "conversationId": change.conversation_id,
        "changeSeq": change.change_seq,
        "messageId": change.message_id,
        "messageVersion": change.message_version,
        "changeKind": change.change_kind,
    })
}

async fn next_text<S>(stream: &mut S) -> Option<String>
where
    S: StreamExt<Item = Result<Message, axum::Error>> + Unpin,
{
    while let Some(item) = stream.next().await {
        match item.ok()? {
            Message::Text(text) => return Some(text.to_string()),
            Message::Ping(_) => continue,
            Message::Close(_) => return None,
            _ => continue,
        }
    }
    None
}

async fn send_json<S>(sink: &mut S, value: &Value) -> Result<(), ()>
where
    S: SinkExt<Message> + Unpin,
{
    let text = serde_json::to_string(value).map_err(|_| ())?;
    sink.send(Message::Text(text.into())).await.map_err(|_| ())
}

/// One socket per signed-in session. It carries "your Inbox changed" and "your channels changed"
/// and nothing else; the client
/// then asks the API what changed, so nothing private travels on the socket.
pub(crate) async fn user_ws(
    State(state): State<AppState>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
    _: AllowedOrigin,
    Actor(user_id): Actor,
) -> Result<axum::response::Response, ApiError> {
    let watch = SessionWatch::new(&state, &headers).await;
    Ok(upgrade
        .max_message_size(EVENT_SOCKET_MAX_MESSAGE)
        .max_frame_size(EVENT_SOCKET_MAX_MESSAGE)
        .on_upgrade(move |socket| handle_user(socket, state, watch, user_id))
        .into_response())
}

async fn handle_user(socket: WebSocket, state: AppState, mut watch: SessionWatch, user_id: Uuid) {
    let (mut sink, mut stream) = socket.split();
    let mut rx = state.changes.subscribe();
    let _ = send_json(&mut sink, &json!({"v":1,"type":"user.ready"})).await;
    loop {
        tokio::select! {
            incoming = stream.next() => match incoming {
                Some(Ok(Message::Ping(payload))) => { if sink.send(Message::Pong(payload)).await.is_err() { return; } },
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                _ => {}
            },
            _ = watch.due() => {
                if watch.ended().await {
                    close_session(&mut sink).await;
                    return;
                }
            },
            event = rx.recv() => match event {
                Ok(ChangeNotice::Inbox { user_id: target }) if target == user_id => {
                    if send_json(&mut sink, &json!({"v":1,"type":"inbox.changed"})).await.is_err() { return; }
                }
                Ok(ChangeNotice::Space { user_id: target }) if target == user_id => {
                    if send_json(&mut sink, &json!({"v":1,"type":"space.changed"})).await.is_err() { return; }
                }
                Ok(ChangeNotice::Access { user_id: target }) if target == user_id => return,
                Ok(_) => {}
                Err(RecvError::Lagged(_)) => {
                    // Missed notices of either kind: both screens look again.
                    if send_json(&mut sink, &json!({"v":1,"type":"inbox.changed"})).await.is_err() { return; }
                    if send_json(&mut sink, &json!({"v":1,"type":"space.changed"})).await.is_err() { return; }
                }
                Err(RecvError::Closed) => return,
            }
        }
    }
}
