//! Resource limits at the edges (FEA-141): an upload nobody may make is refused before its body is read,
//! uploads in flight share a memory budget, event sockets take only small messages, and a ticket
//! description or comment has a size and nesting bound without any change to the shapes accepted.

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use futures_util::{SinkExt, StreamExt};
use penfii_core::files::ObjectStore;
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio_tungstenite::tungstenite::{Message as WsMessage, client::IntoClientRequest};
use uuid::Uuid;

use crate::common::{self, Env, Person, Reply, doc};

struct Rig {
    env: Env,
    /// The router under test, with a small upload budget.
    app: axum::Router,
    ws: Uuid,
}

async fn rig(pool: PgPool, budget_bytes: u64) -> Rig {
    let env = Env::with_objects(pool.clone(), ObjectStore::in_memory()).await;
    let app = penfii_http_api::router_from(penfii_http_api::RouterConfig {
        objects: Some(ObjectStore::in_memory()),
        upload_budget_bytes: Some(budget_bytes),
        upload_wait: Duration::from_millis(400),
        ..penfii_http_api::RouterConfig::new(pool, common::limits(), false)
    });
    let ws = env.ws;
    Rig { env, app, ws }
}

async fn begin(rig: &Rig, who: &Person, size: usize) -> Reply {
    common::call(
        &rig.app,
        "POST",
        &format!("/v1/workspaces/{}/files", rig.ws),
        Some(&who.cookie),
        Some(json!({"filename": "a.bin", "byteSize": size})),
    )
    .await
}

fn put_request(who: &Person, id: &str, declared: usize, body: Body) -> Request<Body> {
    Request::builder()
        .method("PUT")
        .uri(format!("/v1/files/{id}/content"))
        .header("x-penfii-client", "1")
        .header("content-type", "application/octet-stream")
        .header("content-length", declared)
        .header("cookie", &who.cookie)
        .body(body)
        .unwrap()
}

/// A body of `chunks` that counts how many were taken from it.
fn counted(chunks: usize, size: usize) -> (Body, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    let taken = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = taken.clone();
    let stream = futures_util::stream::iter((0..chunks).map(move |_| {
        seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok::<_, std::io::Error>(axum::body::Bytes::from(vec![7u8; size]))
    }));
    (Body::from_stream(stream), taken)
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn an_upload_nobody_may_make_is_refused_before_its_body_is_read(pool: PgPool) {
    let rig = rig(pool, 4 * 1024 * 1024).await;
    let other = rig.env.person("other", "MEMBER").await;
    let created = begin(&rig, &rig.env.owner, 1000).await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    let mine = created.body["id"].as_str().unwrap().to_string();

    for (what, who, id, declared, want) in [
        (
            "no such file",
            &rig.env.owner,
            Uuid::new_v4().to_string(),
            1000,
            StatusCode::NOT_FOUND,
        ),
        (
            "somebody else's file",
            &other,
            mine.clone(),
            1000,
            StatusCode::NOT_FOUND,
        ),
        (
            "another size than begun",
            &rig.env.owner,
            mine.clone(),
            2000,
            StatusCode::CONFLICT,
        ),
    ] {
        let (body, taken) = counted(4, declared / 4);
        let reply = common::send(&rig.app, put_request(who, &id, declared, body)).await;
        assert_eq!(reply.status, want, "{what}: {}", reply.body);
        assert_eq!(
            taken.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "{what}: the body was not read"
        );
    }
    // The rightful upload still works, and a finished one is refused early as stale.
    let (body, _) = counted(4, 250);
    let done = common::send(&rig.app, put_request(&rig.env.owner, &mine, 1000, body)).await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.body);
    let (body, taken) = counted(4, 250);
    let again = common::send(&rig.app, put_request(&rig.env.owner, &mine, 1000, body)).await;
    assert_eq!(again.status, StatusCode::CONFLICT, "{}", again.body);
    assert_eq!(taken.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn uploads_in_flight_share_a_memory_budget_and_the_budget_comes_back(pool: PgPool) {
    // Room for one 8 KiB upload at a time.
    let rig = rig(pool, 8 * 1024).await;
    let first = begin(&rig, &rig.env.owner, 8 * 1024).await.body["id"]
        .as_str()
        .unwrap()
        .to_string();
    let second = begin(&rig, &rig.env.owner, 8 * 1024).await.body["id"]
        .as_str()
        .unwrap()
        .to_string();

    // The first holds the budget: its body is open and has sent half.
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<axum::body::Bytes, std::io::Error>>(4);
    let held = tokio::spawn({
        let app = rig.app.clone();
        let who = rig.env.owner.clone();
        let id = first.clone();
        async move {
            let body = Body::from_stream(tokio_stream_from(rx));
            common::send(&app, put_request(&who, &id, 8 * 1024, body)).await
        }
    });
    tx.send(Ok(axum::body::Bytes::from(vec![1u8; 4096])))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;

    // The second finds the budget used, waits a moment and is told to come back.
    let (body, taken) = counted(8, 1024);
    let busy = common::send(
        &rig.app,
        put_request(&rig.env.owner, &second, 8 * 1024, body),
    )
    .await;
    assert_eq!(
        busy.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        busy.body
    );
    assert_eq!(busy.body["code"], "upload_busy");
    assert_eq!(
        taken.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "its body was not read"
    );

    // The first finishes; the budget is free again and the second goes through.
    tx.send(Ok(axum::body::Bytes::from(vec![1u8; 4096])))
        .await
        .unwrap();
    drop(tx);
    let done = held.await.unwrap();
    assert_eq!(done.status, StatusCode::OK, "{}", done.body);
    let (body, _) = counted(8, 1024);
    let retried = common::send(
        &rig.app,
        put_request(&rig.env.owner, &second, 8 * 1024, body),
    )
    .await;
    assert_eq!(retried.status, StatusCode::OK, "{}", retried.body);

    // An upload that fails mid-way (the client goes away) also gives its bytes back.
    let third = begin(&rig, &rig.env.owner, 8 * 1024).await.body["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<axum::body::Bytes, std::io::Error>>(4);
    tx.send(Ok(axum::body::Bytes::from(vec![1u8; 100])))
        .await
        .unwrap();
    tx.send(Err(std::io::Error::other("connection reset")))
        .await
        .unwrap();
    let failed = common::send(
        &rig.app,
        put_request(
            &rig.env.owner,
            &third,
            8 * 1024,
            Body::from_stream(tokio_stream_from(rx)),
        ),
    )
    .await;
    assert_eq!(failed.status, StatusCode::BAD_REQUEST, "{}", failed.body);
    let (body, _) = counted(8, 1024);
    let after = common::send(
        &rig.app,
        put_request(&rig.env.owner, &third, 8 * 1024, body),
    )
    .await;
    assert_eq!(after.status, StatusCode::OK, "{}", after.body);
}

fn tokio_stream_from(
    mut rx: tokio::sync::mpsc::Receiver<Result<axum::body::Bytes, std::io::Error>>,
) -> impl futures_util::Stream<Item = Result<axum::body::Bytes, std::io::Error>> {
    futures_util::stream::poll_fn(move |cx| rx.poll_recv(cx))
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_large_file_within_the_limit_uploads_with_the_default_budget(pool: PgPool) {
    let env = Env::with_objects(pool, ObjectStore::in_memory()).await;
    let size = 9 * 1024 * 1024;
    let created = common::call(
        &env.app,
        "POST",
        &format!("/v1/workspaces/{}/files", env.ws),
        Some(&env.owner.cookie),
        Some(json!({"filename": "big.bin", "byteSize": size})),
    )
    .await;
    let id = created.body["id"].as_str().unwrap().to_string();
    let (body, _) = counted(9, 1024 * 1024);
    let done = common::send(&env.app, put_request(&env.owner, &id, size, body)).await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.body);
    assert_eq!(done.body["state"], "AVAILABLE");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn an_event_socket_takes_small_messages_only(pool: PgPool) {
    let env = Env::new(pool.clone()).await;
    let app = penfii_http_api::router_from(penfii_http_api::RouterConfig::new(
        pool,
        common::limits(),
        false,
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let connect = || async {
        let mut request = format!("ws://{addr}/v1/ws/user")
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("cookie", env.owner.cookie.parse().unwrap());
        tokio_tungstenite::connect_async(request).await.unwrap().0
    };
    // A hello-sized message is fine: the socket stays open and answers a ping.
    let mut ws = connect().await;
    ws.send(WsMessage::Text(
        json!({"v": 1, "type": "hello", "afterChangeSeq": 0})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    ws.send(WsMessage::Ping(vec![1, 2, 3].into()))
        .await
        .unwrap();
    let mut ponged = false;
    for _ in 0..10 {
        match tokio::time::timeout(Duration::from_secs(2), ws.next()).await {
            Ok(Some(Ok(WsMessage::Pong(_)))) => {
                ponged = true;
                break;
            }
            Ok(Some(Ok(_))) => continue,
            _ => break,
        }
    }
    assert!(ponged, "a small message leaves the socket working");
    // A message of a megabyte is not read: the server ends the connection.
    ws.send(WsMessage::Text("x".repeat(1024 * 1024).into()))
        .await
        .ok();
    let ended = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match ws.next().await {
                None | Some(Err(_)) | Some(Ok(WsMessage::Close(_))) => return true,
                Some(Ok(_)) => continue,
            }
        }
    })
    .await;
    assert_eq!(
        ended,
        Ok(true),
        "the socket is closed after an oversized message"
    );
    server.abort();
}

async fn project_and_ticket(env: &Env) -> (String, String) {
    let project = env
        .call(
            &env.owner,
            "POST",
            &format!("/v1/workspaces/{}/projects", env.ws),
            Some(json!({"name": "Limits", "teamIds": []})),
        )
        .await;
    assert_eq!(project.status, StatusCode::CREATED, "{}", project.body);
    let project = project.body["id"].as_str().unwrap().to_string();
    let ticket = env
        .call(
            &env.owner,
            "POST",
            &format!("/v1/projects/{project}/tickets"),
            Some(json!({"title": "T", "hierarchyKind": "TASK"})),
        )
        .await;
    assert_eq!(ticket.status, StatusCode::CREATED, "{}", ticket.body);
    (project, ticket.body["id"].as_str().unwrap().to_string())
}

fn nested(depth: usize) -> Value {
    let mut node = json!({"type": "paragraph", "content": [{"type": "text", "text": "deep"}]});
    for _ in 0..depth {
        node = json!({"type": "blockquote", "content": [node]});
    }
    json!({"type": "doc", "content": [node]})
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn comments_and_descriptions_have_bounds_and_every_accepted_shape_still_passes(pool: PgPool) {
    let env = Env::new(pool.clone()).await;
    let (project, ticket) = project_and_ticket(&env).await;
    let comment = |body: Value| {
        let env = &env;
        let ticket = ticket.clone();
        async move {
            env.call(
                &env.owner,
                "POST",
                &format!("/v1/tickets/{ticket}/comments"),
                Some(json!({"body": body})),
            )
            .await
        }
    };
    // What MCP writes (plain paragraphs), the web editor's ProseMirror, an unknown node type of an older
    // client and a `penfii.doc` all pass, as before.
    let shapes = [
        doc("from MCP"),
        json!({"type": "doc", "content": [
            {"type": "heading", "attrs": {"level": 2}, "content": [{"type": "text", "text": "H"}]},
            {"type": "paragraph", "content": [{"type": "text", "text": "x", "marks": [{"type": "link", "attrs": {"href": "https://example.com"}}]}]},
            {"type": "customNodeFromAnOlderClient", "attrs": {"a": 1}}]}),
        json!({"schema": "penfii.doc", "version": 1, "blocks": [
            {"id": Uuid::new_v4().to_string(), "type": "paragraph", "props": {}, "content": [{"type": "text", "text": "p", "styles": {}}], "children": []}]}),
        nested(30),
    ];
    let mut first = String::new();
    for shape in shapes {
        let reply = comment(shape.clone()).await;
        assert_eq!(reply.status, StatusCode::CREATED, "{shape}: {}", reply.body);
        if first.is_empty() {
            first = reply.body["id"].as_str().unwrap().to_string();
        }
    }
    // Over the size, or nested without end: refused as a validation error.
    let big = json!({"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "a".repeat(600 * 1024)}]}]});
    for (what, body) in [("too large", big.clone()), ("too deep", nested(200))] {
        let reply = comment(body).await;
        assert_eq!(
            reply.status,
            StatusCode::BAD_REQUEST,
            "{what}: {}",
            reply.body
        );
        assert_eq!(reply.body["code"], "validation");
    }
    // The same for an edit and for a description, on create and on update.
    let edit = env
        .call(
            &env.owner,
            "PATCH",
            &format!("/v1/tickets/{ticket}/comments/{first}"),
            Some(json!({"body": big})),
        )
        .await;
    assert_eq!(edit.status, StatusCode::BAD_REQUEST, "{}", edit.body);
    let long = json!({"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "a".repeat(600 * 1024)}]}]});
    let created = env
        .call(
            &env.owner,
            "POST",
            &format!("/v1/projects/{project}/tickets"),
            Some(json!({"title": "Long", "hierarchyKind": "TASK", "description": long})),
        )
        .await;
    assert_eq!(created.status, StatusCode::BAD_REQUEST, "{}", created.body);
    let ok = env
        .call(
            &env.owner,
            "POST",
            &format!("/v1/projects/{project}/tickets"),
            Some(json!({"title": "Short", "hierarchyKind": "TASK", "description": doc("fine")})),
        )
        .await;
    assert_eq!(ok.status, StatusCode::CREATED, "{}", ok.body);
    // Data stored before the bound (a 600 KiB description) is still read.
    let id: Uuid = ticket.parse().unwrap();
    sqlx::query("UPDATE penfii.tickets SET description = $2 WHERE id = $1")
        .bind(id)
        .bind(&long)
        .execute(&pool)
        .await
        .unwrap();
    let read = env
        .call(&env.owner, "GET", &format!("/v1/tickets/{ticket}"), None)
        .await;
    assert_eq!(read.status, StatusCode::OK, "{}", read.body);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn an_upload_body_that_stalls_is_refused_and_gives_the_budget_back(pool: PgPool) {
    let env = Env::with_objects(pool.clone(), ObjectStore::in_memory()).await;
    // Room for one 8 KiB upload, and a body may be silent for 300 ms.
    let app = penfii_http_api::router_from(penfii_http_api::RouterConfig {
        objects: Some(ObjectStore::in_memory()),
        upload_budget_bytes: Some(8 * 1024),
        upload_wait: Duration::from_millis(400),
        upload_idle: Duration::from_millis(300),
        ..penfii_http_api::RouterConfig::new(pool, common::limits(), false)
    });
    let rig = Rig {
        ws: env.ws,
        env,
        app,
    };
    let id = begin(&rig, &rig.env.owner, 8 * 1024).await.body["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Half the body, then nothing (the sender stays open).
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<axum::body::Bytes, std::io::Error>>(4);
    tx.send(Ok(axum::body::Bytes::from(vec![1u8; 4096])))
        .await
        .unwrap();
    let stalled = tokio::time::timeout(
        Duration::from_secs(5),
        common::send(
            &rig.app,
            put_request(
                &rig.env.owner,
                &id,
                8 * 1024,
                Body::from_stream(tokio_stream_from(rx)),
            ),
        ),
    )
    .await
    .expect("a stalled body is answered, not waited on forever");
    assert_eq!(
        stalled.status,
        StatusCode::REQUEST_TIMEOUT,
        "{}",
        stalled.body
    );
    assert_eq!(stalled.body["code"], "upload_timeout");
    drop(tx);

    // Its share of the budget is back: the whole body goes through at once.
    let (body, _) = counted(8, 1024);
    let done = common::send(&rig.app, put_request(&rig.env.owner, &id, 8 * 1024, body)).await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.body);
}
