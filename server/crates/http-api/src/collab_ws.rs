//! The co-editing socket of a Doc page: `GET /v1/docs/{id}/collab`.
//!
//! Authentication is the session cookie, as for the other sockets. Unlike a plain request, the
//! upgrade carries no `X-Penfii-Client` header and a page on another site of the same registrable
//! domain (or on another port) is "same-site" for `SameSite=Lax`, so this socket, which can change
//! a Doc, also checks the `Origin` the browser reports (see [`crate::origin`]). The page must be readable and
//! editable by that person according to `penfii_core::visibility`, the same function the REST
//! routes use; "no access" is a 404 before the upgrade, so nothing about a restricted page is
//! learned from the answer. The socket itself only moves bytes: rooms, rights and persistence are in
//! `penfii_core::docs::collab`.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use penfii_core::CoreError;
use penfii_core::docs::collab::{CollabHub, JoinError, Outbound, UserInfo, close};
use penfii_core::identity;
use uuid::Uuid;

use crate::AppState;
use crate::error::ApiError;
use crate::extract::{self, CurrentUser};
use crate::origin::AllowedOrigin;

use axum::Router;
use axum::routing::get;

pub(crate) fn routes() -> Router<AppState> {
    Router::new().route("/v1/docs/{id}/collab", get(doc_collab))
}

pub(crate) async fn doc_collab(
    State(state): State<AppState>,
    Path(page): Path<Uuid>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
    _: AllowedOrigin,
    CurrentUser(user): CurrentUser,
) -> Result<Response, ApiError> {
    if !CollabHub::may_open(&state.pool, user.id, page).await? {
        return Err(CoreError::NotFound.into());
    }
    let Some(hub) = state.collab.clone().filter(|hub| hub.holds_lock()) else {
        return Err(ApiError::coded_with(
            StatusCode::SERVICE_UNAVAILABLE,
            "collab_unavailable",
            "co-editing is not available on this server right now",
            serde_json::json!({}),
        ));
    };
    let token = extract::session_token(&headers).unwrap_or_default();
    let info = UserInfo {
        id: user.id,
        name: user.display_name.clone(),
    };
    let pool = state.pool.clone();
    Ok(upgrade
        .on_upgrade(move |socket| pump(socket, hub, pool, info, page, token))
        .into_response())
}

async fn send_close(socket: &mut WebSocket, code: u16, reason: &'static str) {
    let _ = socket
        .send(Message::Close(Some(CloseFrame {
            code,
            reason: reason.into(),
        })))
        .await;
}

async fn pump(
    mut socket: WebSocket,
    hub: Arc<CollabHub>,
    pool: sqlx::PgPool,
    user: UserInfo,
    page: Uuid,
    token: String,
) {
    let mut conn = match hub.join(page, user).await {
        Ok(conn) => conn,
        Err(error) => {
            let (code, reason) = match error {
                JoinError::NoAccess => (close::GONE, "not found"),
                JoinError::TooManyConnections => (close::TOO_MANY, "too many connections"),
                JoinError::TooManyRooms | JoinError::Busy | JoinError::Unavailable => {
                    (close::TRY_LATER, "try again later")
                }
            };
            send_close(&mut socket, code, reason).await;
            return;
        }
    };
    let (mut sink, mut stream) = socket.split();
    let ttl = Duration::from_secs(hub.config().collab_auth_ttl_seconds.max(1));
    let mut ping = tokio::time::interval(Duration::from_secs(25));
    ping.tick().await;
    let mut session_check = tokio::time::interval(ttl.max(Duration::from_secs(5)));
    session_check.tick().await;
    let mut last_heard = tokio::time::Instant::now();
    loop {
        tokio::select! {
            incoming = stream.next() => match incoming {
                Some(Ok(Message::Binary(data))) => {
                    last_heard = tokio::time::Instant::now();
                    conn.frame(data.to_vec());
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(_)) => last_heard = tokio::time::Instant::now(),
            },
            outgoing = conn.rx.recv() => match outgoing {
                Some(Outbound::Binary(data)) => {
                    if sink.send(Message::Binary(data.into())).await.is_err() { break; }
                }
                Some(Outbound::Text(text)) => {
                    if sink.send(Message::Text(text.into())).await.is_err() { break; }
                }
                Some(Outbound::Close(code, reason)) => {
                    let _ = sink.send(Message::Close(Some(CloseFrame { code, reason: reason.into() }))).await;
                    break;
                }
                None => break,
            },
            _ = ping.tick() => {
                if last_heard.elapsed() > Duration::from_secs(60) { break; }
                if sink.send(Message::Ping(Vec::new().into())).await.is_err() { break; }
            }
            _ = session_check.tick() => {
                if identity::authenticate(&pool, &token).await.is_err() {
                    let _ = sink.send(Message::Close(Some(CloseFrame { code: close::SESSION, reason: "session ended".into() }))).await;
                    break;
                }
            }
        }
    }
    // Dropping `conn` leaves the room.
}
