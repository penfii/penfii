//! Conversation socket: the cursor is exclusive, and a live notice cannot skip an earlier committed change.

use super::{World, call, channel, send, workspace};
use futures_util::{SinkExt, StreamExt};
use penfii_core::ChangeNotice;
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::time::{Duration, timeout};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use uuid::Uuid;

struct Socket {
    addr: std::net::SocketAddr,
    changes: tokio::sync::broadcast::Sender<ChangeNotice>,
    server: tokio::task::JoinHandle<()>,
}

impl Socket {
    async fn start(world: &World) -> Socket {
        let (changes, _keep) = tokio::sync::broadcast::channel(64);
        let app = penfii_http_api::router_from(penfii_http_api::RouterConfig {
            changes: Some(changes.clone()),
            ..penfii_http_api::RouterConfig::new(world.pool.clone(), crate::common::limits(), false)
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Socket {
            addr,
            changes,
            server,
        }
    }

    fn stop(self) {
        self.server.abort();
    }
}

async fn open(
    socket: &Socket,
    cookie: &str,
    conversation_id: Uuid,
    after: i64,
) -> WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let mut request = format!(
        "ws://{}/v1/ws?conversationId={conversation_id}",
        socket.addr
    )
    .into_client_request()
    .unwrap();
    request
        .headers_mut()
        .insert("cookie", cookie.parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    ws.send(WsMessage::Text(
        json!({"v": 1, "type": "hello", "afterChangeSeq": after})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    ws
}

async fn next_frame(
    ws: &mut WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    ms: u64,
) -> Option<Value> {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(ms);
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            return None;
        }
        let item = timeout(left, ws.next()).await.ok()??;
        match item.unwrap() {
            WsMessage::Text(text) => return Some(serde_json::from_str(&text).unwrap()),
            WsMessage::Ping(payload) => ws.send(WsMessage::Pong(payload)).await.unwrap(),
            WsMessage::Pong(_) => return Some(json!({"type": "pong"})),
            WsMessage::Close(_) => return None,
            _ => {}
        }
    }
}

async fn change_seqs(
    ws: &mut WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    idle_ms: u64,
) -> Vec<i64> {
    let mut seqs = Vec::new();
    loop {
        match timeout(Duration::from_millis(idle_ms), next_frame(ws, idle_ms)).await {
            Ok(Some(frame)) if frame["type"] == "conversation.change" => {
                seqs.push(frame["changeSeq"].as_i64().unwrap());
            }
            Ok(Some(frame)) if frame["type"] == "pong" => continue,
            _ => break,
        }
    }
    seqs
}

async fn conversation(world: &World) -> (Uuid, i64) {
    let workspace_id = workspace(world, &format!("rt-{}", Uuid::new_v4())).await;
    let conversation_id = channel(world, workspace_id, "general", "PUBLIC").await;
    let (status, _) = send(
        world,
        &world.owner_cookie,
        conversation_id,
        "one",
        None,
        None,
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::CREATED);
    let (status, body, _) = call(
        &world.app,
        "GET",
        &format!("/v1/conversations/{conversation_id}"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK, "{body}");
    (conversation_id, body["lastChangeSeq"].as_i64().unwrap())
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn the_snapshot_cursor_replays_nothing_and_a_later_cursor_replays_only_the_gap(pool: PgPool) {
    let world = World::new(pool).await;
    let (conversation_id, head) = conversation(&world).await;
    assert!(head >= 1);
    let socket = Socket::start(&world).await;

    let mut at_head = open(&socket, &world.owner_cookie, conversation_id, head).await;
    assert!(
        change_seqs(&mut at_head, 200).await.is_empty(),
        "hello at lastChangeSeq does not replay history"
    );
    at_head.send(WsMessage::Ping(vec![1].into())).await.unwrap();
    let pong = next_frame(&mut at_head, 1000).await;
    assert_eq!(pong.unwrap()["type"], "pong");

    let mut behind = open(&socket, &world.owner_cookie, conversation_id, head - 1).await;
    assert_eq!(
        change_seqs(&mut behind, 1000).await,
        vec![head],
        "reconnect continues from the cursor and does not repeat earlier changes"
    );

    socket.stop();
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_live_notice_published_early_still_delivers_the_earlier_change(pool: PgPool) {
    let world = World::new(pool).await;
    let (conversation_id, head) = conversation(&world).await;
    let socket = Socket::start(&world).await;
    let mut ws = open(&socket, &world.owner_cookie, conversation_id, head).await;
    assert!(change_seqs(&mut ws, 200).await.is_empty());
    ws.send(WsMessage::Ping(vec![1].into())).await.unwrap();
    assert_eq!(next_frame(&mut ws, 1000).await.unwrap()["type"], "pong");

    let (status, second) = send(
        &world,
        &world.owner_cookie,
        conversation_id,
        "two",
        None,
        None,
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::CREATED, "{second}");
    let (status, third) = send(
        &world,
        &world.owner_cookie,
        conversation_id,
        "three",
        None,
        None,
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::CREATED, "{third}");
    let (status, listed, _) = call(
        &world.app,
        "GET",
        &format!("/v1/conversations/{conversation_id}/changes?afterSeq={head}"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK, "{listed}");
    let rows = listed.as_array().unwrap();
    assert_eq!(rows.len(), 2);
    let earlier = rows[0]["changeSeq"].as_i64().unwrap();
    let later = rows[1]["changeSeq"].as_i64().unwrap();
    assert!(earlier < later);

    socket
        .changes
        .send(ChangeNotice::Conversation {
            conversation_id,
            change_seq: later,
            message_id: rows[1]["messageId"].as_str().unwrap().parse().unwrap(),
            message_version: 1,
            change_kind: "message.created".to_string(),
        })
        .unwrap();
    assert_eq!(
        change_seqs(&mut ws, 1000).await,
        vec![earlier, later],
        "the earlier committed change is delivered before the one that was published first"
    );

    socket
        .changes
        .send(ChangeNotice::Conversation {
            conversation_id,
            change_seq: earlier,
            message_id: rows[0]["messageId"].as_str().unwrap().parse().unwrap(),
            message_version: 1,
            change_kind: "message.created".to_string(),
        })
        .unwrap();
    assert!(
        change_seqs(&mut ws, 250).await.is_empty(),
        "the caught-up change is not sent a second time"
    );
    socket.stop();
}
