//! Co-editing: a real HTTP server, real WebSockets and a Yjs client written with yrs.

use crate::common::{self};
use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;
// co-editing: a real HTTP server, real WebSockets and a Yjs client written with yrs

use super::docs::{api, make_doc};
use super::inbox::member;
use super::{World, call, invite_member, workspace};
use futures_util::{SinkExt, StreamExt};
use penfii_core::docs::collab::CollabHub;
use penfii_core::docs::yjs as docyjs;
use penfii_core::knowledge::KnowledgeConfig;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use yrs::sync::{Message as YMessage, SyncMessage};
use yrs::types::xml::{XmlOut, XmlTextPrelim};
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{Doc, ReadTxn, Text, Transact, Update, Xml, XmlFragment};

fn collab_config() -> KnowledgeConfig {
    KnowledgeConfig {
        materialize_debounce_ms: 300,
        materialize_max_wait_ms: 1500,
        collab_persist_interval_ms: 100,
        collab_compact_updates: 20,
        collab_room_idle_seconds: 2,
        collab_auth_ttl_seconds: 1,
        ..KnowledgeConfig::default()
    }
}

struct CollabServer {
    /// The same application served on `addr`, for REST calls that must reach this server's rooms.
    app: axum::Router,
    objects: penfii_core::files::ObjectStore,
    addr: std::net::SocketAddr,
    hub: std::sync::Arc<CollabHub>,
    server: tokio::task::JoinHandle<()>,
}

impl CollabServer {
    async fn start(
        world: &World,
        config: KnowledgeConfig,
        lock_key: i64,
        expect_lock: bool,
    ) -> CollabServer {
        let url = common::database_url(&world.pool);
        let hub = CollabHub::with_lock_key(world.pool.clone(), config.clone(), lock_key);
        hub.spawn_owner(url);
        if expect_lock {
            for _ in 0..100 {
                if hub.holds_lock() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            assert!(hub.holds_lock(), "the hub should have taken the lock");
        } else {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        let objects = penfii_core::files::ObjectStore::in_memory();
        let app = penfii_http_api::router_from(penfii_http_api::RouterConfig {
            objects: Some(objects.clone()),
            knowledge: config,
            collab: Some(hub.clone()),
            ..penfii_http_api::RouterConfig::new(world.pool.clone(), common::limits(), false)
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let served = app.clone();
        let server = tokio::spawn(async move {
            axum::serve(listener, served).await.unwrap();
        });
        CollabServer {
            app,
            objects,
            addr,
            hub,
            server,
        }
    }

    async fn api(
        &self,
        method: &str,
        cookie: &str,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let (status, value, _) = call(&self.app, method, path, Some(cookie), None, body).await;
        (status, value)
    }

    /// A graceful stop, as SIGTERM does it.
    async fn stop(self) {
        self.hub.shutdown().await;
        self.server.abort();
    }

    /// The process dies: nothing is stored on the way out.
    async fn crash(self) {
        self.hub.crash_for_tests();
        self.server.abort();
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}

fn lock_key() -> i64 {
    // A key of this test's own: the hubs of different tests must not wait for each other's lock.
    (Uuid::new_v4().as_u128() as i64).abs()
}

struct Editor {
    ws: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    doc: Doc,
    hello: Value,
    controls: Vec<Value>,
    awareness: Vec<(u64, Value)>,
    closed: Option<u16>,
}

#[derive(Debug)]
#[allow(dead_code)] // the payload is for the failure message when an assertion trips
enum Refused {
    Http(u16),
    Closed(Option<u16>),
}

impl Editor {
    async fn open(
        server: &CollabServer,
        cookie: &str,
        page: &str,
        doc: Option<Doc>,
    ) -> Result<Editor, Refused> {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        let mut request = format!("ws://{}/v1/docs/{page}/collab", server.addr)
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("cookie", cookie.parse().unwrap());
        let (ws, _) = match tokio_tungstenite::connect_async(request).await {
            Ok(ok) => ok,
            Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
                return Err(Refused::Http(response.status().as_u16()));
            }
            Err(other) => panic!("{other}"),
        };
        let mut editor = Editor {
            ws,
            doc: doc.unwrap_or_default(),
            hello: Value::Null,
            controls: Vec::new(),
            awareness: Vec::new(),
            closed: None,
        };
        // hello, then the server's state vector; answer with ours and what the server lacks.
        for _ in 0..50 {
            editor.pump(50).await;
            if !editor.hello.is_null() || editor.closed.is_some() {
                break;
            }
        }
        if editor.hello.is_null() {
            return Err(Refused::Closed(editor.closed));
        }
        let sv = editor.doc.transact().state_vector();
        editor
            .send_bin(YMessage::Sync(SyncMessage::SyncStep1(sv)).encode_v1())
            .await;
        editor.pump(100).await;
        Ok(editor)
    }

    async fn send_bin(&mut self, data: Vec<u8>) {
        let _ = self.ws.send(WsMessage::Binary(data.into())).await;
    }

    /// Reads frames for up to `ms` milliseconds (stopping early when the line is quiet).
    async fn pump(&mut self, ms: u64) {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(ms);
        loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            if left.is_zero() {
                break;
            }
            match tokio::time::timeout(left, self.ws.next()).await {
                Err(_) => break,
                Ok(None) => {
                    self.closed.get_or_insert(1006);
                    break;
                }
                Ok(Some(Err(_))) => {
                    self.closed.get_or_insert(1006);
                    break;
                }
                Ok(Some(Ok(WsMessage::Text(text)))) => {
                    let value: Value = serde_json::from_str(&text).unwrap();
                    if value["type"] == "hello" {
                        self.hello = value.clone();
                    }
                    self.controls.push(value);
                }
                Ok(Some(Ok(WsMessage::Binary(data)))) => self.on_binary(&data).await,
                Ok(Some(Ok(WsMessage::Close(frame)))) => {
                    self.closed = Some(frame.map(|f| u16::from(f.code)).unwrap_or(1005));
                    break;
                }
                Ok(Some(Ok(_))) => {}
            }
        }
    }

    async fn on_binary(&mut self, data: &[u8]) {
        let mut decoder = yrs::updates::decoder::DecoderV1::from(data);
        let reader = yrs::sync::MessageReader::new(&mut decoder);
        let messages: Vec<YMessage> = reader.filter_map(Result::ok).collect();
        for message in messages {
            match message {
                YMessage::Sync(SyncMessage::SyncStep1(sv)) => {
                    let update = self.doc.transact().encode_state_as_update_v1(&sv);
                    self.send_bin(YMessage::Sync(SyncMessage::SyncStep2(update)).encode_v1())
                        .await;
                }
                YMessage::Sync(SyncMessage::SyncStep2(bytes))
                | YMessage::Sync(SyncMessage::Update(bytes)) => {
                    let update = Update::decode_v1(&bytes).unwrap();
                    self.doc.transact_mut().apply_update(update).unwrap();
                }
                YMessage::Awareness(update) => {
                    for (id, entry) in update.clients {
                        self.awareness.push((
                            id.get(),
                            serde_json::from_str(&entry.json).unwrap_or(Value::Null),
                        ));
                    }
                }
                _ => {}
            }
        }
    }

    /// Edits the local document and sends the change like a browser would.
    async fn edit(&mut self, change: impl FnOnce(&Doc)) {
        let before = self.doc.transact().state_vector();
        change(&self.doc);
        let update = self.doc.transact().encode_state_as_update_v1(&before);
        self.send_bin(YMessage::Sync(SyncMessage::Update(update)).encode_v1())
            .await;
    }

    async fn type_in(&mut self, block: usize, text: &str) {
        let text = text.to_owned();
        self.edit(move |doc| append_text(doc, block, &text)).await;
    }

    fn text_of(&self) -> Vec<String> {
        block_texts(&self.doc)
    }
}

/// The plain text of each top-level block of the page as the Yjs document holds it.
fn block_texts(doc: &Doc) -> Vec<String> {
    let content = docyjs::materialize(doc, usize::MAX)
        .map(|(c, _)| c)
        .unwrap_or_else(|_| docyjs::to_raw(doc));
    content["blocks"]
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .map(|b| {
                    b["content"]
                        .as_array()
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(|i| i["text"].as_str())
                                .collect::<String>()
                        })
                        .unwrap_or_default()
                })
                .collect()
        })
        .unwrap_or_default()
}

fn nth_container(doc: &Doc, index: usize) -> yrs::XmlElementRef {
    let fragment = doc.get_or_insert_xml_fragment(docyjs::FRAGMENT);
    let txn = doc.transact();
    let group = match fragment.get(&txn, 0) {
        Some(XmlOut::Element(group)) => group,
        other => panic!("no blockGroup: {other:?}"),
    };
    match group.get(&txn, index as u32) {
        Some(XmlOut::Element(container)) => container,
        other => panic!("no block {index}: {other:?}"),
    }
}

fn content_node(doc: &Doc, index: usize) -> yrs::XmlElementRef {
    let container = nth_container(doc, index);
    let txn = doc.transact();
    match container.get(&txn, 0) {
        Some(XmlOut::Element(node)) => node,
        other => panic!("no content node: {other:?}"),
    }
}

/// Appends text to a block's inline text, the way typing at the end does.
fn append_text(doc: &Doc, block: usize, text: &str) {
    let node = content_node(doc, block);
    let existing = {
        let txn = doc.transact();
        match node.get(&txn, 0) {
            Some(XmlOut::Text(t)) => Some(t),
            _ => None,
        }
    };
    let mut txn = doc.transact_mut();
    match existing {
        Some(t) => {
            let at = t.len(&txn);
            t.insert(&mut txn, at, text);
        }
        None => {
            node.push_back(&mut txn, XmlTextPrelim::new(text));
        }
    }
}

async fn page_state(world: &World, page: &str) -> (i64, Value) {
    let row = sqlx::query("SELECT revision_no, content FROM penfii.doc_pages WHERE id=$1")
        .bind(Uuid::parse_str(page).unwrap())
        .fetch_one(&world.pool)
        .await
        .unwrap();
    use sqlx::Row;
    (row.get("revision_no"), row.get("content"))
}

fn plain(content: &Value) -> String {
    penfii_core::docs::content::plain_text(content, 1 << 20).0
}

async fn wait_for<F, Fut>(timeout_ms: u64, mut check: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
    loop {
        if check().await {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn t27_t33_two_editors_converge_and_the_text_reaches_postgresql(pool: PgPool) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("p63-collab-{}", Uuid::new_v4())).await;
    let (_, alice) = member(world, ws, "alice").await;
    let (_, bob) = member(world, ws, "bob").await;
    let page = make_doc(world, &alice, ws, "공동 편집", json!({})).await;
    let id = page["id"].as_str().unwrap().to_owned();
    let server = CollabServer::start(world, collab_config(), lock_key(), true).await;

    let mut a = Editor::open(&server, &alice, &id, None).await.unwrap();
    let mut b = Editor::open(&server, &bob, &id, None).await.unwrap();
    assert_eq!(a.hello["epoch"], b.hello["epoch"], "one lineage per page");
    assert_eq!(a.hello["v"], 1);
    // The server seeded the page; clients never do.
    assert_eq!(a.text_of().len(), 1);

    a.type_in(0, "안녕하세요 ").await;
    b.type_in(0, "world").await;
    a.pump(400).await;
    b.pump(400).await;
    a.pump(200).await;
    assert_eq!(a.text_of(), b.text_of(), "both replicas hold the same text");
    assert!(
        a.text_of()[0].contains("안녕하세요") && a.text_of()[0].contains("world"),
        "{:?}",
        a.text_of()
    );

    // Materialization: debounce 300 ms, so the text is in PostgreSQL shortly, revision_no moved once.
    assert!(
        wait_for(3000, || async {
            plain(&page_state(world, &id).await.1).contains("world")
        })
        .await
    );
    let (revision, content) = page_state(world, &id).await;
    assert_eq!(revision, 2, "one materialization is one revision");
    assert!(plain(&content).contains("안녕하세요"));
    a.pump(300).await;
    assert!(
        a.controls
            .iter()
            .any(|c| c["type"] == "materialized" && c["revisionNo"] == 2),
        "{:?}",
        a.controls
    );
    // The REST view shows the materialized content and says a room is open.
    let (_, rest) = api(world, "GET", &alice, &format!("/v1/docs/{id}"), None).await;
    assert!(plain(&rest["content"]).contains("world"));
    // No more edits: no more revisions.
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    assert_eq!(page_state(world, &id).await.0, 2);
    // The event carries the materialization.
    let events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.domain_events WHERE target_id=$1 AND event_kind='doc.updated'",
    )
    .bind(Uuid::parse_str(&id).unwrap())
    .fetch_one(&world.pool)
    .await
    .unwrap();
    assert_eq!(events, 1);
    drop((a, b));
    server.stop().await;
}

fn delete_block(doc: &Doc, index: u32) {
    let fragment = doc.get_or_insert_xml_fragment(docyjs::FRAGMENT);
    let group = {
        let txn = doc.transact();
        match fragment.get(&txn, 0) {
            Some(XmlOut::Element(group)) => group,
            _ => panic!("no group"),
        }
    };
    let mut txn = doc.transact_mut();
    group.remove_range(&mut txn, index, 1);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn t28_t29_same_block_edits_merge_and_offline_edits_come_back_without_duplicates(
    pool: PgPool,
) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("p63-merge-{}", Uuid::new_v4())).await;
    let (_, alice) = member(world, ws, "alice").await;
    let (_, bob) = member(world, ws, "bob").await;
    let page = make_doc(world, &alice, ws, "병합", json!({"content": {"schema":"penfii.doc","version":1,"blocks":[
        {"id":"p1","type":"paragraph","props":{},"content":[{"type":"text","text":"기준 ","styles":{}}],"children":[]},
        {"id":"p2","type":"paragraph","props":{},"content":[{"type":"text","text":"둘째","styles":{}}],"children":[]}]}})).await;
    let id = page["id"].as_str().unwrap().to_owned();
    let server = CollabServer::start(world, collab_config(), lock_key(), true).await;
    let mut a = Editor::open(&server, &alice, &id, None).await.unwrap();
    let mut b = Editor::open(&server, &bob, &id, None).await.unwrap();
    assert_eq!(a.text_of(), vec!["기준 ", "둘째"]);

    // T28: both type in the same paragraph while one of them deletes the other paragraph.
    a.type_in(0, "A가 씀 ").await;
    b.type_in(0, "B가 씀").await;
    b.edit(|doc| delete_block(doc, 1)).await;
    a.type_in(1, " A가 둘째를 고침").await;
    for _ in 0..3 {
        a.pump(250).await;
        b.pump(250).await;
    }
    assert_eq!(a.text_of(), b.text_of(), "identical on both clients");
    let first = a.text_of()[0].clone();
    assert!(
        first.contains("A가 씀") && first.contains("B가 씀"),
        "both insertions survive: {first}"
    );
    assert!(
        wait_for(3000, || async {
            let (_, c) = page_state(world, &id).await;
            plain(&c).contains("B가 씀")
        })
        .await
    );
    let (_, stored) = page_state(world, &id).await;
    let (mine, _) = docyjs::materialize(&a.doc, usize::MAX).unwrap();
    assert_eq!(
        stored["blocks"], mine["blocks"],
        "the materialized content is what the clients hold"
    );

    // T29: bob loses the connection, keeps typing in memory, and reconnects with the same document.
    let offline_doc = b.doc.clone();
    drop(b);
    append_text(&offline_doc, 0, " [bob offline]");
    a.type_in(0, " [alice meanwhile]").await;
    a.pump(300).await;
    let mut b2 = Editor::open(&server, &bob, &id, Some(offline_doc))
        .await
        .unwrap();
    for _ in 0..4 {
        a.pump(200).await;
        b2.pump(200).await;
    }
    assert_eq!(a.text_of(), b2.text_of());
    let merged = a.text_of()[0].clone();
    assert_eq!(
        merged.matches("[bob offline]").count(),
        1,
        "no duplicate: {merged}"
    );
    assert_eq!(merged.matches("[alice meanwhile]").count(), 1, "{merged}");
    assert_eq!(merged.matches("B가 씀").count(), 1, "{merged}");
    assert_eq!(b2.hello["epoch"], a.hello["epoch"]);
    drop((a, b2));
    server.stop().await;
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn t30_t31_a_restart_keeps_the_text_and_a_crash_before_materialization_is_recovered_once(
    pool: PgPool,
) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("p63-restart-{}", Uuid::new_v4())).await;
    let (_, alice) = member(world, ws, "alice").await;
    let page = make_doc(world, &alice, ws, "재시작", json!({})).await;
    let id = page["id"].as_str().unwrap().to_owned();
    let key = lock_key();
    // A long debounce: nothing reaches doc_pages before the stop/crash, so only the log has it.
    let slow = KnowledgeConfig {
        materialize_debounce_ms: 60_000,
        materialize_max_wait_ms: 120_000,
        ..collab_config()
    };
    let server = CollabServer::start(world, slow.clone(), key, true).await;
    let mut a = Editor::open(&server, &alice, &id, None).await.unwrap();
    let epoch = a.hello["epoch"].clone();
    a.type_in(0, "충돌 전 입력").await;
    a.pump(500).await; // the 100 ms persist interval has passed
    let (rev_before, c) = page_state(world, &id).await;
    assert!(!plain(&c).contains("충돌 전"), "not materialized yet");
    server.crash().await; // T31: the process dies; the log has the update, doc_pages does not
    a.pump(300).await;
    assert!(a.closed.is_some(), "the socket is gone");
    let state = sqlx::query(
        "SELECT last_seq, materialized_seq FROM penfii.doc_collab_state WHERE page_id=$1",
    )
    .bind(Uuid::parse_str(&id).unwrap())
    .fetch_one(&world.pool)
    .await
    .unwrap();
    use sqlx::Row;
    assert!(
        state.get::<i64, _>("last_seq") > state.get::<i64, _>("materialized_seq"),
        "behind the log"
    );

    // A new owner finds the page behind its log and materializes it once, by itself.
    let fast = collab_config();
    let server2 = CollabServer::start(world, fast.clone(), key, true).await;
    assert!(
        wait_for(5000, || async {
            plain(&page_state(world, &id).await.1).contains("충돌 전 입력")
        })
        .await,
        "startup recovery"
    );
    let (rev_after, _) = page_state(world, &id).await;
    assert_eq!(rev_after, rev_before + 1);
    let events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.domain_events WHERE target_id=$1 AND event_kind='doc.updated'",
    )
    .bind(Uuid::parse_str(&id).unwrap())
    .fetch_one(&world.pool)
    .await
    .unwrap();
    assert_eq!(events, 1, "one doc.updated");

    // T30: a client connects after the restart: same lineage, text intact, no duplicate content.
    let mut a2 = Editor::open(&server2, &alice, &id, None).await.unwrap();
    assert_eq!(a2.hello["epoch"], epoch);
    assert_eq!(a2.text_of(), vec!["재시작충돌 전 입력".to_owned()]); // the page started as "재시작"
    a2.type_in(0, " + 재시작 후").await;
    a2.pump(300).await;
    // A graceful stop flushes at once (no debounce wait), and the next process reads it all back.
    drop(a2);
    server2.stop().await;
    let (_, c) = page_state(world, &id).await;
    assert!(
        plain(&c).contains("재시작 후"),
        "shutdown stored the last seconds: {}",
        plain(&c)
    );
    let server3 = CollabServer::start(world, fast, key, true).await;
    let a3 = Editor::open(&server3, &alice, &id, None).await.unwrap();
    assert_eq!(
        a3.text_of(),
        vec!["재시작충돌 전 입력 + 재시작 후".to_owned()]
    );
    drop(a3);
    server3.stop().await;
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn t32_materialization_follows_the_configured_timing(pool: PgPool) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("p63-timing-{}", Uuid::new_v4())).await;
    let (_, alice) = member(world, ws, "alice").await;
    let page = make_doc(world, &alice, ws, "타이밍", json!({})).await;
    let id = page["id"].as_str().unwrap().to_owned();
    // Debounce 1 s, max wait 2.5 s: continuous typing must still reach PostgreSQL by ~2.5 s.
    let cfg = KnowledgeConfig {
        materialize_debounce_ms: 1000,
        materialize_max_wait_ms: 2500,
        ..collab_config()
    };
    let server = CollabServer::start(world, cfg, lock_key(), true).await;
    let mut a = Editor::open(&server, &alice, &id, None).await.unwrap();
    let started = tokio::time::Instant::now();
    let mut seen_at = None;
    let mut i = 0;
    while started.elapsed() < std::time::Duration::from_millis(4500) {
        a.type_in(0, "x").await; // never quiet for a full second
        a.pump(200).await;
        i += 1;
        if seen_at.is_none() && plain(&page_state(world, &id).await.1).contains('x') {
            seen_at = Some(started.elapsed());
        }
    }
    let seen = seen_at.expect("the text reached PostgreSQL while typing continued");
    assert!(
        seen >= std::time::Duration::from_millis(1800)
            && seen <= std::time::Duration::from_millis(3600),
        "max wait bounds it: {seen:?} after {i} edits"
    );
    // The last client leaving materializes at once (not after the quiet period).
    a.type_in(0, "마지막").await;
    a.pump(100).await;
    drop(a);
    assert!(
        wait_for(800, || async {
            plain(&page_state(world, &id).await.1).contains("마지막")
        })
        .await,
        "immediate on leave"
    );
    server.stop().await;
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn t33_revision_no_counts_changing_materializations_only(pool: PgPool) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("p63-rev-{}", Uuid::new_v4())).await;
    let (_, alice) = member(world, ws, "alice").await;
    let page = make_doc(world, &alice, ws, "리비전", json!({})).await;
    let id = page["id"].as_str().unwrap().to_owned();
    let server = CollabServer::start(world, collab_config(), lock_key(), true).await;
    let mut a = Editor::open(&server, &alice, &id, None).await.unwrap();
    // Joining and leaving without a change materializes nothing new.
    a.pump(800).await;
    assert_eq!(page_state(world, &id).await.0, 1);
    a.type_in(0, "하나").await;
    assert!(wait_for(3000, || async { page_state(world, &id).await.0 == 2 }).await);
    a.type_in(0, "둘").await;
    assert!(wait_for(3000, || async { page_state(world, &id).await.0 == 3 }).await);
    // A change that leaves the same content (type, then delete again) is the same hash: no revision.
    let before = a.doc.transact().state_vector();
    append_text(&a.doc, 0, "!!");
    let up1 = a.doc.transact().encode_state_as_update_v1(&before);
    a.send_bin(YMessage::Sync(SyncMessage::Update(up1)).encode_v1())
        .await;
    a.pump(150).await;
    let node = content_node(&a.doc, 0);
    let text = {
        let txn = a.doc.transact();
        match node.get(&txn, 0) {
            Some(XmlOut::Text(t)) => t,
            _ => panic!(),
        }
    };
    let before = a.doc.transact().state_vector();
    {
        let mut txn = a.doc.transact_mut();
        let len = text.len(&txn);
        text.remove_range(&mut txn, len - 2, 2);
    }
    let up2 = a.doc.transact().encode_state_as_update_v1(&before);
    a.send_bin(YMessage::Sync(SyncMessage::Update(up2)).encode_v1())
        .await;
    a.pump(1200).await;
    assert_eq!(
        page_state(world, &id).await.0,
        3,
        "back to the same text: same hash, same revision"
    );
    // A title change is a revision; tree moves are not. REST never compares revision_no.
    let (s, v) = server
        .api(
            "PATCH",
            &alice,
            &format!("/v1/docs/{id}"),
            Some(json!({"title":"새 제목"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["revisionNo"], 4);
    a.pump(300).await;
    assert!(
        a.controls
            .iter()
            .any(|c| c["type"] == "meta" && c["title"] == "새 제목"),
        "{:?}",
        a.controls
    );
    drop(a);
    server.stop().await;
}

async fn deactivate(world: &World, ws: Uuid, user: Uuid) {
    sqlx::query("UPDATE penfii.workspace_members SET state='INACTIVE', deactivated_at=now() WHERE workspace_id=$1 AND user_id=$2")
        .bind(ws).bind(user).execute(&world.pool).await.unwrap();
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn gap_the_collab_socket_refuses_a_foreign_origin(pool: PgPool) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("p63-origin-{}", Uuid::new_v4())).await;
    let owner = world.owner_cookie.clone();
    let server = CollabServer::start(world, collab_config(), lock_key(), true).await;
    let page = make_doc(world, &owner, ws, "출처", json!({})).await;
    let id = page["id"].as_str().unwrap().to_owned();
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let attempt = |origin: Option<String>| {
        let url = format!("ws://{}/v1/docs/{id}/collab", server.addr);
        let cookie = owner.clone();
        async move {
            let mut request = url.into_client_request().unwrap();
            request
                .headers_mut()
                .insert("cookie", cookie.parse().unwrap());
            if let Some(origin) = origin {
                request
                    .headers_mut()
                    .insert("origin", origin.parse().unwrap());
            }
            match tokio_tungstenite::connect_async(request).await {
                Ok(_) => 101,
                Err(tokio_tungstenite::tungstenite::Error::Http(r)) => r.status().as_u16(),
                Err(other) => panic!("{other}"),
            }
        }
    };
    // a page of another site, with a valid session cookie, is refused before anything else
    assert_eq!(attempt(Some("https://evil.example.org".into())).await, 403);
    assert_eq!(attempt(Some("null".into())).await, 403);
    assert_eq!(
        attempt(Some(format!("http://{}.evil.org", server.addr))).await,
        403
    );
    // the server's own origin and a client that is not a browser are accepted
    assert_eq!(attempt(Some(format!("http://{}", server.addr))).await, 101);
    assert_eq!(attempt(None).await, 101);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn every_live_socket_refuses_a_foreign_origin(pool: PgPool) {
    let world = &World::new(pool).await;
    let owner = world.owner_cookie.clone();
    let server = CollabServer::start(world, collab_config(), lock_key(), true).await;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let unknown = Uuid::new_v4();
    let paths = [
        format!("/v1/ws?conversationId={unknown}"),
        "/v1/ws/user".to_owned(),
        format!("/v1/ws/projects/{unknown}"),
        format!("/v1/docs/{unknown}/collab"),
    ];
    for path in paths {
        let attempt = |origin: Option<String>| {
            let url = format!("ws://{}{path}", server.addr);
            let cookie = owner.clone();
            async move {
                let mut request = url.into_client_request().unwrap();
                request
                    .headers_mut()
                    .insert("cookie", cookie.parse().unwrap());
                if let Some(origin) = origin {
                    request
                        .headers_mut()
                        .insert("origin", origin.parse().unwrap());
                }
                match tokio_tungstenite::connect_async(request).await {
                    Ok(_) => 101,
                    Err(tokio_tungstenite::tungstenite::Error::Http(r)) => r.status().as_u16(),
                    Err(other) => panic!("{other}"),
                }
            }
        };
        // A page of another site that rides on the session cookie never gets past the handshake,
        // whatever the socket would have said about the (unknown) id.
        assert_eq!(
            attempt(Some("https://evil.example.org".into())).await,
            403,
            "{path}"
        );
        assert_eq!(attempt(Some("null".into())).await, 403, "{path}");
        // The server's own origin and a client without an Origin reach the socket's own checks.
        assert_ne!(
            attempt(Some(format!("http://{}", server.addr))).await,
            403,
            "{path}"
        );
        assert_ne!(attempt(None).await, 403, "{path}");
    }
    server.stop().await;
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn t34_t35_t36_t37_the_socket_follows_the_same_permissions_as_the_api(pool: PgPool) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("p63-perm-{}", Uuid::new_v4())).await;
    let owner = world.owner_cookie.clone();
    let (_, admin) = invite_member(world, ws, "ADMIN").await;
    let (bob_id, bob) = member(world, ws, "bob").await;
    let (carol_id, carol) = member(world, ws, "carol").await;
    let (dan_id, dan) = member(world, ws, "dan").await;
    let server = CollabServer::start(world, collab_config(), lock_key(), true).await;

    // T34: a restricted page. carol manages it, dan is an editor; bob, the Workspace owner and
    // the admin are not on it. The socket answers them like the REST route: not found.
    let page = make_doc(
        world,
        &carol,
        ws,
        "비밀",
        json!({
        "visibility":"RESTRICTED","participants":[{"userId": dan_id, "role":"EDITOR"}]}),
    )
    .await;
    let id = page["id"].as_str().unwrap().to_owned();
    for (who, cookie) in [("bob", &bob), ("owner", &owner), ("admin", &admin)] {
        let r = Editor::open(&server, cookie, &id, None).await;
        assert!(matches!(r, Err(Refused::Http(404))), "{who}: {:?}", r.err());
    }
    let r = Editor::open(&server, "penfii_session=nope", &id, None).await;
    assert!(matches!(r, Err(Refused::Http(401))), "{:?}", r.err());
    let mut c = Editor::open(&server, &carol, &id, None).await.unwrap();
    let mut d = Editor::open(&server, &dan, &id, None).await.unwrap();
    c.type_in(0, "비밀 문장").await;
    d.pump(400).await;
    assert!(d.text_of()[0].contains("비밀 문장"));

    // T35: carol removes dan while he is connected. His socket closes with 4403 within a second,
    // he gets the `revoked` frame first, and what he tries to send afterwards never lands.
    let (s, v) = server
        .api(
            "PUT",
            &carol,
            &format!("/v1/docs/{id}/participants"),
            Some(json!({"participants":[{"userId": carol_id, "role":"MANAGER"}]})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    d.pump(1000).await;
    assert_eq!(d.closed, Some(4403), "closed by the revocation hook");
    assert!(d.controls.iter().any(|c| c["type"] == "revoked"));
    d.type_in(0, " 몰래").await; // the socket is gone; nothing arrives
    c.pump(600).await;
    assert!(!c.text_of()[0].contains("몰래"));
    assert!(
        matches!(
            Editor::open(&server, &dan, &id, None).await,
            Err(Refused::Http(404))
        ),
        "reconnect is a 404"
    );

    // T36: an open page with two people connected is made restricted: only the one who is still a
    // participant keeps the socket.
    let open = make_doc(world, &carol, ws, "열림", json!({})).await;
    let open_id = open["id"].as_str().unwrap().to_owned();
    let mut ob = Editor::open(&server, &bob, &open_id, None).await.unwrap();
    let mut oc = Editor::open(&server, &carol, &open_id, None).await.unwrap();
    let (s, v) = server
        .api(
            "PUT",
            &carol,
            &format!("/v1/docs/{open_id}/visibility"),
            Some(json!({"visibility":"RESTRICTED"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    ob.pump(1200).await;
    oc.pump(300).await;
    assert_eq!(ob.closed, Some(4403), "bob is not a participant");
    assert_eq!(oc.closed, None, "carol is, and stays");
    oc.type_in(0, "남은 사람").await;
    oc.pump(200).await;
    assert!(
        wait_for(3000, || async {
            plain(&page_state(world, &open_id).await.1).contains("남은 사람")
        })
        .await
    );

    // T37: a member is deactivated while connected. Edits sent after the check interval (1 s here)
    // are not applied, and the socket closes.
    let shared = make_doc(world, &carol, ws, "공유", json!({})).await;
    let shared_id = shared["id"].as_str().unwrap().to_owned();
    let mut sb = Editor::open(&server, &bob, &shared_id, None).await.unwrap();
    let mut sc = Editor::open(&server, &carol, &shared_id, None)
        .await
        .unwrap();
    deactivate(world, ws, bob_id).await;
    tokio::time::sleep(std::time::Duration::from_millis(2200)).await;
    sb.type_in(0, " 비활성 후").await;
    sb.pump(600).await;
    sc.pump(600).await;
    assert_eq!(sb.closed, Some(4403));
    assert!(!sc.text_of()[0].contains("비활성 후"), "{:?}", sc.text_of());
    drop((c, d, ob, oc, sb, sc));
    server.stop().await;
}

fn awareness_frame(entries: Vec<(u64, u32, Value)>) -> Vec<u8> {
    use yrs::sync::awareness::{AwarenessUpdate, AwarenessUpdateEntry};
    let mut clients = std::collections::HashMap::new();
    for (id, clock, state) in entries {
        clients.insert(
            yrs::ClientID::new(id),
            AwarenessUpdateEntry {
                clock,
                json: state.to_string().into(),
            },
        );
    }
    YMessage::Awareness(AwarenessUpdate { clients }).encode_v1()
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn t38_t39_t42_awareness_cannot_be_forged_limits_hold_and_a_second_instance_serves_no_rooms(
    pool: PgPool,
) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("p63-limits-{}", Uuid::new_v4())).await;
    let (_, alice) = member(world, ws, "alice").await;
    let (bob_id, bob) = member(world, ws, "bob").await;
    let page = make_doc(world, &alice, ws, "한도", json!({})).await;
    let id = page["id"].as_str().unwrap().to_owned();
    let cfg = KnowledgeConfig {
        collab_max_update_bytes: 4_000,
        collab_max_state_bytes: 12_000,
        ..collab_config()
    };
    let key = lock_key();
    let server = CollabServer::start(world, cfg.clone(), key, true).await;
    let mut a = Editor::open(&server, &alice, &id, None).await.unwrap();
    let mut b = Editor::open(&server, &bob, &id, None).await.unwrap();

    // T38: bob claims to be "관리자" and sets a cursor; alice sees bob's real name and only the cursor.
    b.send_bin(awareness_frame(vec![(
        777,
        1,
        json!({"user":{"name":"관리자","color":"#000"},"cursor":{"anchor":1},"admin":true}),
    )]))
    .await;
    a.pump(500).await;
    let seen: Vec<&Value> = a
        .awareness
        .iter()
        .filter(|(id, _)| *id == 777)
        .map(|(_, v)| v)
        .collect();
    assert!(!seen.is_empty(), "alice receives bob's cursor");
    for state in &seen {
        assert_eq!(
            state["user"]["name"], "Viewer Two",
            "the server wrote the name: {state}"
        );
        assert_eq!(state["user"]["userId"], json!(bob_id));
        assert!(
            state.get("admin").is_none(),
            "unknown fields are dropped: {state}"
        );
        assert_eq!(state["cursor"]["anchor"], 1);
    }
    // He cannot speak for alice's awareness id once she has one.
    a.send_bin(awareness_frame(vec![(
        888,
        1,
        json!({"cursor":{"anchor":2}}),
    )]))
    .await;
    a.pump(300).await;
    b.send_bin(awareness_frame(vec![(
        888,
        9,
        json!({"cursor":{"anchor":99}}),
    )]))
    .await;
    a.pump(300).await;
    let forged = a
        .awareness
        .iter()
        .any(|(id, v)| *id == 888 && v["cursor"]["anchor"] == 99);
    assert!(!forged, "an id owned by another connection is ignored");

    // T39: an update over the limit closes that sender (4413); the room carries on for the others.
    b.edit(|doc| append_text(doc, 0, &"가".repeat(2_500))).await; // > 4 000 bytes in UTF-8
    b.pump(600).await;
    assert_eq!(b.closed, Some(4413));
    a.type_in(0, "계속").await;
    a.pump(200).await;
    assert!(a.text_of()[0].contains("계속"));
    // The state limit: many small updates until the page would grow past it.
    let mut c = Editor::open(&server, &bob, &id, None).await.unwrap();
    let mut refused = false;
    for _ in 0..40 {
        c.type_in(0, &"x".repeat(1_500)).await;
        c.pump(30).await;
        if c.closed == Some(4413) {
            refused = true;
            break;
        }
    }
    assert!(refused, "the state limit closes the sender with 4413");
    a.pump(300).await;
    assert!(a.closed.is_none(), "the room is unaffected");

    // T42: another instance does not hold the lock: no rooms, but REST works.
    let second = CollabServer::start(world, cfg, key, false).await;
    assert!(!second.hub.holds_lock());
    assert!(matches!(
        Editor::open(&second, &alice, &id, None).await,
        Err(Refused::Http(503))
    ));
    let (s, v) = second
        .api("GET", &alice, &format!("/v1/docs/{id}"), None)
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    drop((a, b, c));
    second.stop().await;
    server.stop().await;
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn t40_a_forged_structure_is_refused_at_materialization_and_an_operator_reset_starts_a_new_lineage(
    pool: PgPool,
) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("p63-invalid-{}", Uuid::new_v4())).await;
    let (_, alice) = member(world, ws, "alice").await;
    let (_, bob) = member(world, ws, "bob").await;
    let page = make_doc(world, &alice, ws, "위조", json!({})).await;
    let id = page["id"].as_str().unwrap().to_owned();
    let page_uuid = Uuid::parse_str(&id).unwrap();
    let server = CollabServer::start(world, collab_config(), lock_key(), true).await;
    let mut a = Editor::open(&server, &alice, &id, None).await.unwrap();
    let mut b = Editor::open(&server, &bob, &id, None).await.unwrap();
    a.type_in(0, "좋은 내용").await;
    assert!(
        wait_for(3000, || async {
            plain(&page_state(world, &id).await.1).contains("좋은 내용")
        })
        .await
    );
    let (rev, good) = page_state(world, &id).await;
    let old_epoch = a.hello["epoch"].clone();

    // A modified client writes a block type the editor does not offer.
    b.pump(300).await;
    b.edit(|doc| {
        use yrs::types::xml::XmlElementPrelim;
        let fragment = doc.get_or_insert_xml_fragment(docyjs::FRAGMENT);
        let group = {
            let txn = doc.transact();
            match fragment.get(&txn, 0) {
                Some(XmlOut::Element(g)) => g,
                _ => panic!(),
            }
        };
        let mut txn = doc.transact_mut();
        let container = group.push_back(&mut txn, XmlElementPrelim::empty("blockContainer"));
        container.insert_attribute(&mut txn, "id", "evil".to_owned());
        container.push_back(&mut txn, XmlElementPrelim::empty("toggleListItem"));
    })
    .await;
    b.pump(2500).await;
    let invalid: bool = sqlx::query_scalar(
        "SELECT invalid_since IS NOT NULL FROM penfii.doc_collab_state WHERE page_id=$1",
    )
    .bind(page_uuid)
    .fetch_one(&world.pool)
    .await
    .unwrap();
    assert!(invalid, "invalid_since is set");
    let (rev_now, content_now) = page_state(world, &id).await;
    assert_eq!(
        (rev_now, &content_now),
        (rev, &good),
        "the last good content stays"
    );
    assert!(!content_now.to_string().contains("evil"));

    // The operator resets the lineage from the last good content (what `doc-collab-reset` does).
    let new_epoch = penfii_core::docs::collab::store::reset(&world.pool, page_uuid)
        .await
        .unwrap();
    a.pump(2500).await;
    b.pump(500).await;
    assert_eq!(a.closed, Some(1012), "every client is told to reconnect");
    assert_eq!(b.closed, Some(1012));
    let again = Editor::open(&server, &alice, &id, None).await.unwrap();
    assert_ne!(again.hello["epoch"], old_epoch);
    assert_eq!(again.hello["epoch"], json!(new_epoch));
    assert!(
        again.text_of()[0].contains("좋은 내용") && again.text_of().len() == 1,
        "{:?}",
        again.text_of()
    );
    let invalid: bool = sqlx::query_scalar(
        "SELECT invalid_since IS NOT NULL FROM penfii.doc_collab_state WHERE page_id=$1",
    )
    .bind(page_uuid)
    .fetch_one(&world.pool)
    .await
    .unwrap();
    assert!(!invalid);
    drop((a, b, again));
    server.stop().await;
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn t43_a_file_in_a_doc_is_readable_by_that_docs_readers_and_only_while_it_is_referenced(
    pool: PgPool,
) {
    let world = &World::new(pool).await;
    let ws = workspace(world, &format!("p63-files-{}", Uuid::new_v4())).await;
    let (alice_id, alice) = member(world, ws, "alice").await;
    let (bob_id, bob) = member(world, ws, "bob").await;
    let (_, carol) = member(world, ws, "carol").await;
    let server = CollabServer::start(world, collab_config(), lock_key(), true).await;

    let file_id = Uuid::new_v4();
    let key = format!("p63/{file_id}");
    sqlx::query("INSERT INTO penfii.files(id,workspace_id,storage_key,filename,byte_size,detected_mime,state,created_by) VALUES($1,$2,$3,'도면.png',5,'image/png','AVAILABLE',$4)")
        .bind(file_id).bind(ws).bind(&key).bind(alice_id).execute(&world.pool).await.unwrap();
    server.objects.put(&key, b"bytes").await.unwrap();
    let path = format!("/v1/files/{file_id}/content");

    // Before it is in a Doc it is alice's pending upload: only she can read it.
    assert_eq!(
        server.api("GET", &alice, &path, None).await.0,
        StatusCode::OK
    );
    assert_eq!(
        server.api("GET", &bob, &path, None).await.0,
        StatusCode::NOT_FOUND
    );

    // A restricted page where bob is an editor and carol is nobody.
    let page = make_doc(
        world,
        &alice,
        ws,
        "도면",
        json!({"visibility":"RESTRICTED","participants":[{"userId": bob_id, "role":"EDITOR"}]}),
    )
    .await;
    let id = page["id"].as_str().unwrap().to_owned();
    let (s, v) = server
        .api(
            "POST",
            &alice,
            &format!("/v1/docs/{id}/files"),
            Some(json!({"fileId": file_id})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    // Others may not attach a file that is not theirs.
    let (s, _) = server
        .api(
            "POST",
            &bob,
            &format!("/v1/docs/{id}/files"),
            Some(json!({"fileId": file_id})),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // The attach call is what makes it available to the room before any materialization.
    assert_eq!(
        server.api("GET", &bob, &path, None).await.0,
        StatusCode::OK,
        "a collaborator reads it at once"
    );
    assert_eq!(
        server.api("GET", &alice, &path, None).await.0,
        StatusCode::OK
    );
    assert_eq!(
        server.api("GET", &carol, &path, None).await.0,
        StatusCode::NOT_FOUND,
        "not a reader of the page"
    );
    // alice loses the page; the uploader has no claim of her own any more.
    let (s, v) = server
        .api(
            "PUT",
            &alice,
            &format!("/v1/docs/{id}/participants"),
            Some(json!({"participants":[{"userId": bob_id, "role":"MANAGER"}]})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(
        server.api("GET", &alice, &path, None).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(server.api("GET", &bob, &path, None).await.0, StatusCode::OK);

    // The materializer keeps a file node only if the file was attached on purpose, and detaches
    // a file no node refers to after the grace period.
    let cfg = KnowledgeConfig {
        doc_file_orphan_grace_seconds: 1,
        ..collab_config()
    };
    let other = Uuid::new_v4(); // a file id the page never attached
    let with_files = json!({"schema":"penfii.doc","version":1,"blocks":[
        {"id":"i1","type":"image","props":{"url": penfii_core::docs::content::file_url(file_id),"name":"도면.png"},"children":[]},
        {"id":"i2","type":"image","props":{"url": penfii_core::docs::content::file_url(other),"name":"남의 파일.png"},"children":[]},
        {"id":"p","type":"paragraph","content":[{"type":"text","text":"본문"}],"children":[]}]});
    let applied = penfii_core::docs::materialize::apply(
        &world.pool,
        &cfg,
        Uuid::parse_str(&id).unwrap(),
        &with_files,
        &[bob_id],
        None,
    )
    .await
    .unwrap();
    assert!(matches!(
        applied,
        penfii_core::docs::materialize::Applied::Written { .. }
    ));
    let (_, stored) = page_state(world, &id).await;
    let urls: Vec<String> = stored["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|b| b["props"]["url"].as_str().map(str::to_owned))
        .collect();
    assert_eq!(
        urls,
        vec![penfii_core::docs::content::file_url(file_id)],
        "the unattached file's block was dropped"
    );
    // Now nothing refers to the file: it is marked, then detached after the grace period.
    sqlx::query(
        "UPDATE penfii.doc_page_files SET attached_at = now() - interval '1 hour' WHERE page_id=$1",
    )
    .bind(Uuid::parse_str(&id).unwrap())
    .execute(&world.pool)
    .await
    .unwrap();
    let without = json!({"schema":"penfii.doc","version":1,"blocks":[{"id":"p","type":"paragraph","content":[{"type":"text","text":"사진 뺌"}],"children":[]}]});
    penfii_core::docs::materialize::apply(
        &world.pool,
        &cfg,
        Uuid::parse_str(&id).unwrap(),
        &without,
        &[bob_id],
        None,
    )
    .await
    .unwrap();
    let marked: bool = sqlx::query_scalar("SELECT unreferenced_since IS NOT NULL FROM penfii.doc_page_files WHERE page_id=$1 AND file_id=$2").bind(Uuid::parse_str(&id).unwrap()).bind(file_id).fetch_one(&world.pool).await.unwrap();
    assert!(marked);
    assert_eq!(
        server.api("GET", &bob, &path, None).await.0,
        StatusCode::OK,
        "still attached during the grace period"
    );
    tokio::time::sleep(std::time::Duration::from_millis(1300)).await;
    let without2 = json!({"schema":"penfii.doc","version":1,"blocks":[{"id":"p","type":"paragraph","content":[{"type":"text","text":"한 번 더"}],"children":[]}]});
    penfii_core::docs::materialize::apply(
        &world.pool,
        &cfg,
        Uuid::parse_str(&id).unwrap(),
        &without2,
        &[bob_id],
        None,
    )
    .await
    .unwrap();
    let left: i64 =
        sqlx::query_scalar("SELECT count(*) FROM penfii.doc_page_files WHERE page_id=$1")
            .bind(Uuid::parse_str(&id).unwrap())
            .fetch_one(&world.pool)
            .await
            .unwrap();
    assert_eq!(left, 0, "detached");
    // Detached means it is nobody's but the uploader's pending upload again.
    assert_eq!(
        server.api("GET", &bob, &path, None).await.0,
        StatusCode::NOT_FOUND
    );
    server.stop().await;
}
