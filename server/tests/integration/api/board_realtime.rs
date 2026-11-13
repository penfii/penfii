//! The Project Board socket and the user socket: what reaches an open Board, what does not, and what
//! happens when a person loses access or a socket falls behind. Every wait is for a frame with a time
//! limit; "does not arrive" is shown by a visible change published afterwards that arrives first.

use super::tickets::state_id;
use super::{World, call, grant, make_ticket, new_project, user_id_of, viewer_cookie};
use axum::http::StatusCode;
use futures_util::{SinkExt, StreamExt};
use penfii_core::ChangeNotice;
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::time::{Duration, timeout};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use uuid::Uuid;

type Ws = WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// The router with a broadcast of its own that the sockets listen to, served on a free port. HTTP
/// commands go through `app` (the same state), so what a handler publishes reaches the sockets.
struct Live {
    app: axum::Router,
    addr: std::net::SocketAddr,
    changes: tokio::sync::broadcast::Sender<ChangeNotice>,
    server: tokio::task::JoinHandle<()>,
}

impl Live {
    async fn start(world: &World, capacity: usize) -> Live {
        let (changes, _keep) = tokio::sync::broadcast::channel(capacity);
        let app = penfii_http_api::router_from(penfii_http_api::RouterConfig {
            changes: Some(changes.clone()),
            ..penfii_http_api::RouterConfig::new(world.pool.clone(), crate::common::limits(), false)
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let serve = app.clone();
        let server = tokio::spawn(async move {
            axum::serve(listener, serve).await.unwrap();
        });
        Live {
            app,
            addr,
            changes,
            server,
        }
    }

    fn stop(self) {
        self.server.abort();
    }
}

async fn connect(live: &Live, cookie: &str, path: &str) -> Ws {
    let mut request = format!("ws://{}{path}", live.addr)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("cookie", cookie.parse().unwrap());
    let (ws, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    ws
}

async fn open_board(live: &Live, cookie: &str, project_id: &str) -> Ws {
    let mut ws = connect(live, cookie, &format!("/v1/ws/projects/{project_id}")).await;
    ws.send(WsMessage::Text(
        json!({"v": 1, "type": "hello", "afterChangeSeq": 0})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    let ready = next_frame(&mut ws).await.expect("project.ready");
    assert_eq!(ready["type"], "project.ready", "{ready}");
    ws
}

/// Opens the board socket with a position (what the client saw last): every frame before `project.ready`, then that frame.
async fn open_board_after(
    live: &Live,
    cookie: &str,
    project_id: &str,
    after: i64,
) -> (Ws, Vec<Value>, Value) {
    let mut ws = connect(live, cookie, &format!("/v1/ws/projects/{project_id}")).await;
    ws.send(WsMessage::Text(
        json!({"v": 1, "type": "hello", "afterChangeSeq": after})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    let mut before = Vec::new();
    loop {
        let frame = next_frame(&mut ws).await.expect("project.ready");
        if frame["type"] == "project.ready" {
            return (ws, before, frame);
        }
        before.push(frame);
    }
}

async fn open_user(live: &Live, cookie: &str) -> Ws {
    let mut ws = connect(live, cookie, "/v1/ws/user").await;
    let ready = next_frame(&mut ws).await.expect("user.ready");
    assert_eq!(ready["type"], "user.ready", "{ready}");
    ws
}

/// The next text frame, or `None` when the socket closed or nothing came within the limit.
async fn next_frame(ws: &mut Ws) -> Option<Value> {
    loop {
        match timeout(Duration::from_secs(10), ws.next()).await.ok()?? {
            Ok(WsMessage::Text(text)) => return Some(serde_json::from_str(&text).unwrap()),
            Ok(WsMessage::Ping(payload)) => ws.send(WsMessage::Pong(payload)).await.unwrap(),
            Ok(WsMessage::Close(_)) | Err(_) => return None,
            Ok(_) => {}
        }
    }
}

/// `(changeKind, ticketId)` of the next ticket change frame.
async fn next_change(ws: &mut Ws) -> (String, String) {
    let frame = next_frame(ws).await.expect("a frame");
    assert_eq!(frame["type"], "project.ticket.change", "{frame}");
    (
        frame["changeKind"].as_str().unwrap().to_owned(),
        frame["ticketId"].as_str().unwrap().to_owned(),
    )
}

async fn act(live: &Live, cookie: &str, path: &str, key: &str, body: Value) -> (StatusCode, Value) {
    let (status, value, _) =
        call(&live.app, "POST", path, Some(cookie), Some(key), Some(body)).await;
    (status, value)
}

/// A command through the world's own router: it publishes to nobody, so setup leaves no frames behind.
async fn quiet(
    world: &World,
    cookie: &str,
    path: &str,
    key: &str,
    body: Value,
) -> (StatusCode, Value) {
    let (status, value, _) = call(
        &world.app,
        "POST",
        path,
        Some(cookie),
        Some(key),
        Some(body),
    )
    .await;
    (status, value)
}

/// Frames until a `project.resync_required` comes (other frames of the same command may come first).
async fn until_resync(ws: &mut Ws) -> Value {
    for _ in 0..10 {
        let frame = next_frame(ws).await.expect("a frame");
        if frame["type"] == "project.resync_required" {
            return frame;
        }
    }
    panic!("no project.resync_required");
}

async fn ticket(world: &World, project: &str, key: &str, body: Value) -> String {
    let (status, value) = make_ticket(world, project, key, body).await;
    assert_eq!(status, StatusCode::CREATED, "{value}");
    value["id"].as_str().unwrap().to_owned()
}

struct Setup {
    project: String,
    workflow: Value,
    viewer: String,
    open: String,
    hidden: String,
}

/// A Project with a person who only reads it, an ordinary ticket and a ticket in a private channel that
/// person is not in.
async fn setup(world: &World, name: &str) -> Setup {
    let (project, workflow) = new_project(world, world.workspace_id, name).await;
    let email = format!("{name}-viewer@penfii.test");
    let viewer = viewer_cookie(world, world.workspace_id, &email).await;
    grant(world, &project, user_id_of(world, &email).await, "VIEWER").await;
    let (status, channel, _) = call(
        &world.app,
        "POST",
        &format!("/v1/workspaces/{}/channels", world.workspace_id),
        Some(&world.owner_cookie),
        None,
        Some(json!({"name":format!("{name}-secret"),"visibility":"PRIVATE","teamId":null,"projectId":project})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{channel}");
    let open = ticket(world, &project, "br-open", json!({"title":"Open"})).await;
    let hidden = ticket(
        world,
        &project,
        "br-hidden",
        json!({"title":"Hidden","channelId":channel["id"]}),
    )
    .await;
    Setup {
        project,
        workflow,
        viewer,
        open,
        hidden,
    }
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn archive_trash_restore_and_purge_reach_the_people_who_can_see_the_ticket(pool: PgPool) {
    let world = &World::new(pool).await;
    let s = setup(world, "brl").await;
    let live = Live::start(world, 64).await;
    let owner = world.owner_cookie.clone();
    let mut admin_ws = open_board(&live, &owner, &s.project).await;
    let mut viewer_ws = open_board(&live, &s.viewer, &s.project).await;
    let id = &s.open;

    for (n, (command, kind)) in [
        ("archive", "ticket.archived"),
        ("unarchive", "ticket.unarchived"),
        ("trash", "ticket.trashed"),
        ("restore-from-trash", "ticket.restored_from_trash"),
        ("trash", "ticket.trashed"),
        ("purge", "ticket.purged"),
    ]
    .into_iter()
    .enumerate()
    {
        let (status, body) = act(
            &live,
            &owner,
            &format!("/v1/tickets/{id}/commands/{command}"),
            &format!("br-{n}"),
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{command}: {body}");
        for ws in [&mut admin_ws, &mut viewer_ws] {
            assert_eq!(
                next_change(ws).await,
                (kind.to_owned(), id.clone()),
                "{command}"
            );
        }
    }

    // A ticket in a private channel the viewer is not in: the admin is told, the viewer is not. A
    // visible change published afterwards arrives first for the viewer, which shows nothing came before it.
    let other = ticket(world, &s.project, "br-other", json!({"title":"Other"})).await;
    let hidden = &s.hidden;
    let (status, body) = act(
        &live,
        &owner,
        &format!("/v1/tickets/{hidden}/commands/archive"),
        "br-h1",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (status, body) = act(
        &live,
        &owner,
        &format!("/v1/tickets/{other}/commands/archive"),
        "br-h2",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert_eq!(
        next_change(&mut admin_ws).await,
        ("ticket.archived".into(), hidden.clone())
    );
    assert_eq!(
        next_change(&mut admin_ws).await,
        ("ticket.archived".into(), other.clone())
    );
    assert_eq!(
        next_change(&mut viewer_ws).await,
        ("ticket.archived".into(), other.clone()),
        "the viewer's first change is the visible one"
    );

    // Archiving what is already archived changes nothing and tells nobody; a later change is the next frame.
    let (status, _) = act(
        &live,
        &owner,
        &format!("/v1/tickets/{other}/commands/archive"),
        "br-h3",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = act(
        &live,
        &owner,
        &format!("/v1/tickets/{other}/commands/unarchive"),
        "br-h4",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        next_change(&mut viewer_ws).await,
        ("ticket.unarchived".into(), other)
    );

    live.stop();
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn bulk_archive_and_trash_reach_open_boards(pool: PgPool) {
    let world = &World::new(pool).await;
    let s = setup(world, "brb").await;
    let live = Live::start(world, 64).await;
    let owner = world.owner_cookie.clone();
    let mut viewer_ws = open_board(&live, &s.viewer, &s.project).await;
    let second = ticket(world, &s.project, "brb-2", json!({"title":"Two"})).await;
    let workspace = world.workspace_id;

    for (n, (change, kind)) in [("ARCHIVE", "ticket.archived"), ("TRASH", "ticket.trashed")]
        .into_iter()
        .enumerate()
    {
        // The trash change needs a live ticket, so it takes the other one.
        let ids = if change == "ARCHIVE" {
            vec![s.open.clone(), s.hidden.clone()]
        } else {
            vec![second.clone()]
        };
        let (status, body) = act(
            &live,
            &owner,
            &format!("/v1/workspaces/{workspace}/tickets/bulk"),
            &format!("brb-{n}"),
            json!({"ticketIds": ids, "change": {"type": change}}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let expected: Vec<String> = if change == "ARCHIVE" {
            vec![s.open.clone()] // the hidden one is not announced to this viewer
        } else {
            vec![second.clone()]
        };
        for id in expected {
            assert_eq!(next_change(&mut viewer_ws).await, (kind.to_owned(), id));
        }
    }
    live.stop();
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn completing_a_sprint_with_an_archive_tells_the_board_to_look_again(pool: PgPool) {
    let world = &World::new(pool).await;
    let s = setup(world, "brs").await;
    let done = state_id(&s.workflow, "DONE");
    let live = Live::start(world, 64).await;
    let owner = world.owner_cookie.clone();
    let mut viewer_ws = open_board(&live, &s.viewer, &s.project).await;

    let mut sprints = Vec::new();
    for (n, (starts, ends)) in [("2026-11-02", "2026-11-13"), ("2026-11-16", "2026-11-27")]
        .into_iter()
        .enumerate()
    {
        let (status, value, _) = call(
            &world.app,
            "POST",
            &format!("/v1/projects/{}/sprints", s.project),
            Some(&owner),
            Some(&format!("brs-s{n}")),
            Some(json!({"name":format!("S{n}"),"startsOn":starts,"endsOn":ends})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{value}");
        sprints.push(value["id"].as_str().unwrap().to_owned());
    }
    // One finished task per sprint: the first is archived with the completion, the second on schedule.
    let mut tasks = Vec::new();
    for (n, sprint) in sprints.iter().enumerate() {
        let task = ticket(
            world,
            &s.project,
            &format!("brs-t{n}"),
            json!({"title":format!("Task {n}"),"hierarchyKind":"TASK"}),
        )
        .await;
        let (status, value) = quiet(
            world,
            &owner,
            &format!("/v1/tickets/{task}/commands/assign-sprint"),
            &format!("brs-as{n}"),
            json!({"sprintId":sprint}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{value}");
        let (status, value) = quiet(
            world,
            &owner,
            &format!("/v1/sprints/{sprint}/commands/start"),
            &format!("brs-st{n}"),
            json!({"expectedVersion":1}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{value}");
        let (status, value) = quiet(
            world,
            &owner,
            &format!("/v1/tickets/{task}/commands/change-status"),
            &format!("brs-d{n}"),
            json!({"expectedRevision":1,"statusId":done}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{value}");
        tasks.push(task);
    }
    // Immediate: one frame for the Project, not one per ticket.
    let (status, value) = act(
        &live,
        &owner,
        &format!("/v1/sprints/{}/commands/complete", sprints[0]),
        "brs-c0",
        json!({"expectedVersion":2,"archive":"IMMEDIATE"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    let frame = until_resync(&mut viewer_ws).await;
    assert_eq!(frame["projectId"], json!(s.project));

    // Scheduled: the worker's pass returns the notice and the process publishes it.
    let at = (chrono::Utc::now() + chrono::Duration::hours(2)).to_rfc3339();
    let (status, value) = act(
        &live,
        &owner,
        &format!("/v1/sprints/{}/commands/complete", sprints[1]),
        "brs-c1",
        json!({"expectedVersion":2,"archive":"AT","archiveAt":at}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    sqlx::query("UPDATE penfii.sprint_archive_schedules SET due_at = now() - interval '1 hour'")
        .execute(&world.pool)
        .await
        .unwrap();
    let (handled, notices) = penfii_core::planning::run_due_notices(&world.pool, 10)
        .await
        .unwrap();
    assert_eq!(handled, 1);
    assert_eq!(notices.len(), 1);
    for notice in notices {
        live.changes.send(notice).unwrap();
    }
    until_resync(&mut viewer_ws).await;

    live.stop();
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_board_socket_ends_when_access_is_lost_and_asks_for_a_resync_when_it_falls_behind(
    pool: PgPool,
) {
    let world = &World::new(pool).await;
    let s = setup(world, "bra").await;
    // A tiny buffer, so a burst without a pause in between makes the socket lag for certain.
    let live = Live::start(world, 4).await;
    let viewer_id = user_id_of(world, "bra-viewer@penfii.test").await;

    // A notice about another Project is not this board's.
    let mut ws = open_board(&live, &s.viewer, &s.project).await;
    live.changes
        .send(ChangeNotice::Ticket {
            project_id: Uuid::new_v4(),
            change_seq: 1001,
            ticket_id: Uuid::new_v4(),
            ticket_revision_no: 1,
            change_kind: "ticket.updated".into(),
        })
        .unwrap();
    live.changes
        .send(ChangeNotice::Ticket {
            project_id: s.project.parse().unwrap(),
            change_seq: 1002,
            ticket_id: s.open.parse().unwrap(),
            ticket_revision_no: 1,
            change_kind: "ticket.updated".into(),
        })
        .unwrap();
    assert_eq!(
        next_change(&mut ws).await,
        ("ticket.updated".into(), s.open.clone())
    );

    // Access lost: the socket closes.
    live.changes
        .send(ChangeNotice::Access { user_id: viewer_id })
        .unwrap();
    assert!(next_frame(&mut ws).await.is_none());

    // Falling behind: a burst larger than the buffer, sent without yielding.
    let mut ws = open_board(&live, &s.viewer, &s.project).await;
    for seq in 0..32 {
        live.changes
            .send(ChangeNotice::Ticket {
                project_id: s.project.parse().unwrap(),
                change_seq: 1000 + seq,
                ticket_id: s.open.parse().unwrap(),
                ticket_revision_no: 1,
                change_kind: "ticket.updated".into(),
            })
            .unwrap();
    }
    let mut seen_resync = false;
    while let Some(frame) = next_frame(&mut ws).await {
        if frame["type"] == "project.resync_required" {
            seen_resync = true;
        }
    }
    assert!(
        seen_resync,
        "the socket asks the client to read the board again, then closes"
    );
    live.stop();
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn the_user_socket_carries_only_the_persons_own_notices(pool: PgPool) {
    let world = &World::new(pool).await;
    let live = Live::start(world, 4).await;
    let owner = world.owner_id;
    let mut ws = open_user(&live, &world.owner_cookie).await;

    // Someone else's notices, then the person's own: the first frame is the own one.
    live.changes
        .send(ChangeNotice::Inbox {
            user_id: Uuid::new_v4(),
        })
        .unwrap();
    live.changes
        .send(ChangeNotice::Space {
            user_id: Uuid::new_v4(),
        })
        .unwrap();
    live.changes
        .send(ChangeNotice::Inbox { user_id: owner })
        .unwrap();
    assert_eq!(next_frame(&mut ws).await.unwrap()["type"], "inbox.changed");
    live.changes
        .send(ChangeNotice::Space { user_id: owner })
        .unwrap();
    assert_eq!(next_frame(&mut ws).await.unwrap()["type"], "space.changed");

    // Falling behind: both screens are told to look again.
    for _ in 0..32 {
        live.changes
            .send(ChangeNotice::Project {
                project_id: Uuid::new_v4(),
            })
            .unwrap();
    }
    let types = [
        next_frame(&mut ws).await.unwrap()["type"].clone(),
        next_frame(&mut ws).await.unwrap()["type"].clone(),
    ];
    assert_eq!(types, [json!("inbox.changed"), json!("space.changed")]);

    // Access lost: the socket closes.
    live.changes
        .send(ChangeNotice::Access { user_id: owner })
        .unwrap();
    assert!(next_frame(&mut ws).await.is_none());
    live.stop();
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_reconnecting_board_socket_is_caught_up_from_the_change_feed_without_hidden_tickets(
    pool: PgPool,
) {
    let world = &World::new(pool).await;
    let s = setup(world, "brr").await;
    let live = Live::start(world, 64).await;
    let owner = world.owner_cookie.clone();

    // The first connection has no position: no frames, and `project.ready` names the head of the feed.
    let (ws, before, ready) = open_board_after(&live, &s.viewer, &s.project, 0).await;
    assert!(before.is_empty(), "{before:?}");
    let seen = ready["headSeq"].as_i64().expect("headSeq");
    assert!(seen >= 2, "the setup wrote changes: {ready}");
    drop(ws);

    // While nobody listens: a visible ticket and a hidden ticket change.
    for (n, id) in [&s.open, &s.hidden].into_iter().enumerate() {
        let (status, body) = quiet(
            world,
            &owner,
            &format!("/v1/tickets/{id}/commands/archive"),
            &format!("brr-{n}"),
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    }

    // The viewer is caught up with the visible change only, in order, then told the head.
    let (_ws, frames, ready) = open_board_after(&live, &s.viewer, &s.project, seen).await;
    assert_eq!(frames.len(), 1, "{frames:?}");
    assert_eq!(frames[0]["type"], "project.ticket.change");
    assert_eq!(frames[0]["ticketId"], json!(s.open));
    assert_eq!(frames[0]["changeKind"], "ticket.archived");
    assert!(frames[0]["changeSeq"].as_i64().unwrap() > seen);
    assert_eq!(ready["headSeq"].as_i64().unwrap(), seen + 2);
    // The admin sees both.
    let (_ws, frames, _) = open_board_after(&live, &owner, &s.project, seen).await;
    let ids: Vec<_> = frames
        .iter()
        .map(|frame| frame["ticketId"].clone())
        .collect();
    assert_eq!(ids, vec![json!(s.open), json!(s.hidden)]);

    // A live change carries its position too.
    let (mut ws, _, ready) = open_board_after(&live, &s.viewer, &s.project, seen + 2).await;
    assert_eq!(ready["headSeq"].as_i64().unwrap(), seen + 2);
    let (status, body) = act(
        &live,
        &owner,
        &format!("/v1/tickets/{}/commands/unarchive", s.open),
        "brr-u",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let frame = next_frame(&mut ws).await.expect("a live change");
    assert_eq!(frame["changeSeq"].as_i64().unwrap(), seen + 3, "{frame}");
    live.stop();
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_position_the_feed_cannot_continue_from_means_read_the_project_again(pool: PgPool) {
    let world = &World::new(pool).await;
    let s = setup(world, "brn").await;
    let live = Live::start(world, 64).await;

    // Ahead of the feed (a restored database, another server): the client cannot trust what it holds.
    let (_ws, before, ready) = open_board_after(&live, &s.viewer, &s.project, 10_000).await;
    assert_eq!(before.len(), 1, "{before:?}");
    assert_eq!(before[0]["type"], "project.resync_required");
    assert_eq!(before[0]["projectId"], json!(s.project));
    // The ready frame still gives the head, so the client starts from it after reading the Project.
    assert!(ready["headSeq"].as_i64().unwrap() < 10_000);

    // Too far behind to replay: more changes than the socket replays.
    for n in 0..520 {
        sqlx::query(
            "INSERT INTO penfii.project_ticket_changes(project_id,change_seq,ticket_id,ticket_revision_no,change_kind)
             SELECT $1, coalesce(max(change_seq),0)+1, $2, 1, 'ticket.updated' FROM penfii.project_ticket_changes WHERE project_id=$1",
        )
        .bind(s.project.parse::<Uuid>().unwrap())
        .bind(s.open.parse::<Uuid>().unwrap())
        .execute(&world.pool)
        .await
        .unwrap_or_else(|error| panic!("{n}: {error}"));
    }
    let (_ws, before, _) = open_board_after(&live, &s.viewer, &s.project, 1).await;
    assert_eq!(before.len(), 1, "{}", before.len());
    assert_eq!(before[0]["type"], "project.resync_required");
    live.stop();
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn changes_no_ticket_record_carries_tell_the_board_to_read_the_project_again(pool: PgPool) {
    let world = &World::new(pool).await;
    let s = setup(world, "brp").await;
    let live = Live::start(world, 64).await;
    let owner = world.owner_cookie.clone();
    let mut ws = open_board(&live, &s.viewer, &s.project).await;

    async fn expect_resync(ws: &mut Ws, what: &str) {
        let frame = next_frame(ws)
            .await
            .unwrap_or_else(|| panic!("{what}: no frame"));
        assert_eq!(frame["type"], "project.resync_required", "{what}: {frame}");
    }
    let (status, sprint) = act(
        &live,
        &owner,
        &format!("/v1/projects/{}/sprints", s.project),
        "brp-s",
        json!({"name":"S","startsOn":"2026-11-02","endsOn":"2026-11-13"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{sprint}");
    expect_resync(&mut ws, "create sprint").await;
    let sprint_id = sprint["id"].as_str().unwrap().to_owned();
    let (status, body, _) = call(
        &live.app,
        "PATCH",
        &format!("/v1/sprints/{sprint_id}"),
        Some(&owner),
        None,
        Some(json!({"expectedVersion":1,"name":"Renamed"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    expect_resync(&mut ws, "update sprint").await;
    let (status, body) = act(
        &live,
        &owner,
        &format!("/v1/sprints/{sprint_id}/commands/start"),
        "brp-st",
        json!({"expectedVersion":2}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    expect_resync(&mut ws, "start sprint").await;
    let (status, milestone) = act(
        &live,
        &owner,
        &format!("/v1/projects/{}/milestones", s.project),
        "brp-m",
        json!({"name":"Beta"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{milestone}");
    expect_resync(&mut ws, "create milestone").await;
    let milestone_id = milestone["id"].as_str().unwrap();
    let (status, body, _) = call(
        &live.app,
        "PATCH",
        &format!("/v1/milestones/{milestone_id}"),
        Some(&owner),
        None,
        Some(json!({"expectedVersion":1,"name":"Beta 2"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    expect_resync(&mut ws, "update milestone").await;
    let done = state_id(&s.workflow, "DONE");
    let (status, body, _) = call(
        &live.app,
        "PATCH",
        &format!("/v1/projects/{}/workflow/states/{done}", s.project),
        Some(&owner),
        None,
        Some(json!({"expectedWorkflowVersion": s.workflow["version"], "wipLimit": 5})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    expect_resync(&mut ws, "workflow state").await;
    live.stop();
}
