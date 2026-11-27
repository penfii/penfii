//! The socket of an open Doc page and what feeds it: which changes tell an open page to read again, who is
//! told, and that nothing of the page travels on it. Every wait is for a frame with a time limit; "does not
//! arrive" is shown by a visible change published afterwards that arrives first.

use super::board_realtime::{Live, Ws, connect, next_frame};
use super::docs::{api, make_doc, save_doc};
use super::inbox::member;
use super::{World, workspace};
use axum::http::StatusCode;
use futures_util::SinkExt;
use penfii_core::ChangeNotice;
use serde_json::json;
use sqlx::PgPool;
use tokio::sync::broadcast;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use uuid::Uuid;

/// What a person who may not open the socket is answered: the status of the refused upgrade.
async fn refused(live: &Live, cookie: Option<&str>, path: &str) -> StatusCode {
    let mut request = format!("ws://{}{path}", live.addr)
        .into_client_request()
        .unwrap();
    if let Some(cookie) = cookie {
        request
            .headers_mut()
            .insert("cookie", cookie.parse().unwrap());
    }
    match tokio_tungstenite::connect_async(request).await {
        Err(tokio_tungstenite::tungstenite::Error::Http(response)) => response.status(),
        other => panic!(
            "expected the upgrade to be refused, got {:?}",
            other.map(|_| ())
        ),
    }
}

async fn open_doc(live: &Live, cookie: &str, id: &str) -> Ws {
    let mut ws = connect(live, cookie, &format!("/v1/ws/docs/{id}")).await;
    let ready = next_frame(&mut ws).await.expect("doc.ready");
    assert_eq!(ready["type"], "doc.ready", "{ready}");
    assert_eq!(ready["docId"], id);
    ws
}

fn tell(live: &Live, id: &str) {
    live.changes
        .send(ChangeNotice::Doc {
            page_id: Uuid::parse_str(id).unwrap(),
        })
        .unwrap();
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn an_open_page_is_told_to_read_again_and_only_when_it_may(pool: PgPool) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("doc-live-{}", Uuid::new_v4())).await;
    let owner = world.owner_cookie.clone();
    let (reader_id, reader) = member(world, ws, "reader").await;
    let (_, outsider) = member(world, ws, "outsider").await;
    let page = make_doc(
        world,
        &owner,
        ws,
        "Live",
        json!({"visibility": "RESTRICTED", "participants": [{"userId": reader_id, "role": "VIEWER"}]}),
    )
    .await;
    let id = page["id"].as_str().unwrap().to_owned();
    let other = make_doc(world, &owner, ws, "Elsewhere", json!({})).await;
    let other_id = other["id"].as_str().unwrap().to_owned();
    let live = Live::start(world, 64).await;

    // Who may open it: a reader and the manager; a member who cannot read the page, someone without a session
    // and a page that does not exist get what the page itself answers, with nothing said about the page.
    let mut reader_ws = open_doc(&live, &reader, &id).await;
    let mut owner_ws = open_doc(&live, &owner, &id).await;
    assert_eq!(
        refused(&live, Some(&outsider), &format!("/v1/ws/docs/{id}")).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        refused(&live, None, &format!("/v1/ws/docs/{id}")).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        refused(
            &live,
            Some(&reader),
            &format!("/v1/ws/docs/{}", Uuid::new_v4())
        )
        .await,
        StatusCode::NOT_FOUND
    );

    // Another page's change is not this page's: the notice for this page is the first frame.
    tell(&live, &other_id);
    tell(&live, &id);
    for socket in [&mut reader_ws, &mut owner_ws] {
        let frame = next_frame(socket).await.expect("doc.changed");
        assert_eq!(frame, json!({"v": 1, "type": "doc.changed", "docId": id}));
    }

    // The frame names the page and carries nothing of it.
    let owner_frame = {
        tell(&live, &id);
        next_frame(&mut owner_ws).await.unwrap()
    };
    assert_eq!(owner_frame.as_object().unwrap().len(), 3, "{owner_frame}");
    next_frame(&mut reader_ws).await.unwrap();

    // The reader loses the page: one frame with nothing in it, then the socket ends. The manager's stays.
    let (status, body) = api(
        world,
        "PUT",
        &owner,
        &format!("/v1/docs/{id}/participants"),
        Some(json!({"participants": [{"userId": world.owner_id, "role": "MANAGER"}]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    tell(&live, &id);
    let gone = next_frame(&mut reader_ws).await.expect("doc.gone");
    assert_eq!(gone, json!({"v": 1, "type": "doc.gone", "docId": id}));
    assert!(
        next_frame(&mut reader_ws).await.is_none(),
        "the socket ends"
    );
    assert_eq!(
        next_frame(&mut owner_ws).await.unwrap()["type"],
        "doc.changed"
    );
    // A page that was deleted ends the manager's socket the same way.
    let (status, _) = api(world, "DELETE", &owner, &format!("/v1/docs/{id}"), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    tell(&live, &id);
    let gone = next_frame(&mut owner_ws).await.expect("doc.gone");
    assert_eq!(gone["type"], "doc.gone");
    assert!(next_frame(&mut owner_ws).await.is_none());
    live.stop();
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_socket_that_missed_notices_is_told_to_read(pool: PgPool) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("doc-lag-{}", Uuid::new_v4())).await;
    let owner = world.owner_cookie.clone();
    let page = make_doc(world, &owner, ws, "Busy", json!({})).await;
    let id = page["id"].as_str().unwrap().to_owned();
    // A broadcast of two: a burst that nobody reads overruns it.
    let live = Live::start(world, 2).await;
    let mut socket = open_doc(&live, &owner, &id).await;
    for _ in 0..20 {
        tell(&live, &id);
    }
    // Whatever was missed, the socket says to read; reading once more is all a client does.
    let frame = next_frame(&mut socket).await.expect("a frame");
    assert_eq!(frame["type"], "doc.changed", "{frame}");

    // Frames the client sends are not commands: the socket goes on telling.
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            json!({"v": 1, "type": "hello"}).to_string().into(),
        ))
        .await
        .unwrap();
    // The burst is drained (more than one frame may be waiting); a notice after it is the next frame.
    while !live.changes.is_empty() {
        tokio::task::yield_now().await;
    }
    let mut last = None;
    for _ in 0..5 {
        tell(&live, &id);
        last = next_frame(&mut socket).await;
        if last.as_ref().is_some_and(|f| f["type"] == "doc.changed") {
            break;
        }
    }
    assert_eq!(last.expect("a frame")["type"], "doc.changed");
    live.stop();
}

/// The consumer: which events become a notice, so every writer (Save, API, MCP, the width, access, Versions,
/// deletion) is covered by one rule and the rest (comments, moves) are not.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn the_changes_a_reader_sees_become_notices_and_the_others_do_not(pool: PgPool) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("doc-notice-{}", Uuid::new_v4())).await;
    let owner = world.owner_cookie.clone();
    let (reader_id, reader) = member(world, ws, "reader").await;
    let page = make_doc(world, &owner, ws, "Notices", json!({})).await;
    let id = page["id"].as_str().unwrap().to_owned();
    let pid = Uuid::parse_str(&id).unwrap();
    let (sender, mut receiver) = broadcast::channel(64);
    let mut registry = penfii_core::consumers::registry(None, Some(sender));
    registry.workspace = Some(ws);

    async fn notices(
        world: &World,
        registry: &penfii_core::events::Registry,
        receiver: &mut broadcast::Receiver<ChangeNotice>,
    ) -> Vec<Uuid> {
        penfii_core::events::run_until_idle(&world.pool, registry)
            .await
            .unwrap();
        let mut pages = Vec::new();
        while let Ok(notice) = receiver.try_recv() {
            if let ChangeNotice::Doc { page_id } = notice {
                pages.push(page_id);
            }
        }
        pages
    }
    // What setup recorded is not the subject.
    notices(world, &registry, &mut receiver).await;

    // The editor's Save.
    let (status, saved) = save_doc(world, &owner, &id, 1, "Notices", "first").await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(
        notices(world, &registry, &mut receiver).await,
        vec![pid],
        "save"
    );
    // The title alone.
    let (status, _) = api(
        world,
        "PATCH",
        &owner,
        &format!("/v1/docs/{id}"),
        Some(json!({"title": "Renamed"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        notices(world, &registry, &mut receiver).await,
        vec![pid],
        "title"
    );
    // The width: no revision, and still a change a reader sees. Saying what it already is changes nothing.
    let (status, wide) = api(
        world,
        "PATCH",
        &owner,
        &format!("/v1/docs/{id}"),
        Some(json!({"fullWidth": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{wide}");
    assert_eq!(
        notices(world, &registry, &mut receiver).await,
        vec![pid],
        "width"
    );
    let (status, _) = api(
        world,
        "PATCH",
        &owner,
        &format!("/v1/docs/{id}"),
        Some(json!({"fullWidth": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        notices(world, &registry, &mut receiver).await.is_empty(),
        "no change of the width, no notice"
    );
    // A Version.
    let (_, current) = api(world, "GET", &owner, &format!("/v1/docs/{id}"), None).await;
    let (status, version) = api(
        world,
        "POST",
        &owner,
        &format!("/v1/docs/{id}/versions"),
        Some(json!({"expectedRevisionNo": current["revisionNo"]})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{version}");
    assert_eq!(
        notices(world, &registry, &mut receiver).await,
        vec![pid],
        "version"
    );
    // Who may read it.
    let (status, _) = api(
        world,
        "PUT",
        &owner,
        &format!("/v1/docs/{id}/visibility"),
        Some(json!({"visibility": "RESTRICTED"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        notices(world, &registry, &mut receiver).await,
        vec![pid],
        "visibility"
    );
    let (status, _) = api(
        world,
        "PUT",
        &owner,
        &format!("/v1/docs/{id}/participants"),
        Some(json!({"participants": [{"userId": world.owner_id, "role": "MANAGER"}, {"userId": reader_id, "role": "VIEWER"}]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        notices(world, &registry, &mut receiver).await,
        vec![pid],
        "participants"
    );

    // Comments, and what comes with them, are not the page.
    let (status, comment) = api(
        world,
        "POST",
        &reader,
        &format!("/v1/docs/{id}/comments"),
        Some(json!({"body": {"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"hi"}]}]}})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{comment}");
    assert!(
        notices(world, &registry, &mut receiver).await.is_empty(),
        "comment"
    );

    // The deletion.
    let (status, _) = api(world, "DELETE", &owner, &format!("/v1/docs/{id}"), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        notices(world, &registry, &mut receiver).await,
        vec![pid],
        "deletion"
    );
}
