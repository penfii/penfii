//! Real-time co-editing of Doc pages.
//!
//! `CollabHub` owns the rooms of one core-api process. Rooms live in memory, so only one instance
//! may own them: at start the hub takes a session-level PostgreSQL advisory lock on a dedicated
//! connection and keeps trying to take it while another instance holds it. An instance without the
//! lock serves everything else normally and answers co-editing upgrades with `collab_unavailable`.
//!
//! The Yjs state is a recoverable working copy. `doc_pages.content` is what the application reads,
//! and it is written only by the materializer (`docs::materialize`).

mod room;
pub mod store;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use sqlx::{Connection, PgConnection, PgPool};
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use self::room::{JoinDenied, RoomCmd};
use crate::knowledge::KnowledgeConfig;
use crate::visibility;

pub use self::room::{JoinInfo, color_of};

/// Fixed key of the advisory lock that makes one process the owner of all rooms.
pub const COLLAB_LOCK_KEY: i64 = 0x7065_6e66_6969_0001;

/// WebSocket close codes (section F.3).
pub mod close {
    pub const GOING_AWAY: u16 = 1001;
    pub const RESTART: u16 = 1012;
    pub const TRY_LATER: u16 = 1013;
    pub const BAD_MESSAGE: u16 = 4400;
    pub const SESSION: u16 = 4401;
    pub const REVOKED: u16 = 4403;
    pub const GONE: u16 = 4404;
    pub const TOO_LARGE: u16 = 4413;
    pub const TOO_MANY: u16 = 4429;
}

/// What a room sends to one connection.
#[derive(Debug)]
pub enum Outbound {
    Binary(Vec<u8>),
    Text(String),
    Close(u16, &'static str),
}

#[derive(Debug, Clone)]
pub struct UserInfo {
    pub id: Uuid,
    pub name: String,
}

#[derive(Debug)]
pub enum JoinError {
    /// This process does not own the rooms (another instance does, or co-editing is off).
    Unavailable,
    NoAccess,
    TooManyRooms,
    TooManyConnections,
    Busy,
}

#[derive(Clone)]
struct RoomHandle {
    token: u64,
    tx: mpsc::UnboundedSender<RoomCmd>,
}

pub struct CollabHub {
    pool: PgPool,
    config: Arc<KnowledgeConfig>,
    rooms: Mutex<HashMap<Uuid, RoomHandle>>,
    /// Open connections per (user, page), for the per-user cap.
    connections: Mutex<HashMap<(Uuid, Uuid), usize>>,
    lock_key: i64,
    holds_lock: AtomicBool,
    shutting_down: AtomicBool,
    next_conn: AtomicU64,
    next_token: AtomicU64,
}

/// One accepted connection. Dropping it leaves the room.
pub struct CollabConnection {
    pub id: u64,
    pub epoch: Uuid,
    pub rx: mpsc::UnboundedReceiver<Outbound>,
    sender: mpsc::UnboundedSender<RoomCmd>,
    hub: Arc<CollabHub>,
    user: Uuid,
    page: Uuid,
}

impl CollabConnection {
    pub fn frame(&self, data: Vec<u8>) {
        let _ = self.sender.send(RoomCmd::Frame {
            conn: self.id,
            data,
        });
    }
}

impl Drop for CollabConnection {
    fn drop(&mut self) {
        let _ = self.sender.send(RoomCmd::Leave { conn: self.id });
        self.hub.release(self.user, self.page);
    }
}

impl CollabHub {
    pub fn new(pool: PgPool, config: KnowledgeConfig) -> Arc<Self> {
        Self::with_lock_key(pool, config, COLLAB_LOCK_KEY)
    }

    /// A hub that competes for another advisory lock. Tests give each independent server its own,
    /// since they share one database; production always uses [`COLLAB_LOCK_KEY`].
    pub fn with_lock_key(pool: PgPool, config: KnowledgeConfig, lock_key: i64) -> Arc<Self> {
        Arc::new(Self {
            lock_key,
            pool,
            config: Arc::new(config),
            rooms: Mutex::new(HashMap::new()),
            connections: Mutex::new(HashMap::new()),
            holds_lock: AtomicBool::new(false),
            shutting_down: AtomicBool::new(false),
            next_conn: AtomicU64::new(1),
            next_token: AtomicU64::new(1),
        })
    }

    pub fn holds_lock(&self) -> bool {
        self.holds_lock.load(Ordering::SeqCst) && !self.shutting_down.load(Ordering::SeqCst)
    }

    pub fn config(&self) -> &KnowledgeConfig {
        &self.config
    }

    pub fn is_open(&self, page: Uuid) -> bool {
        self.rooms
            .lock()
            .map(|r| r.contains_key(&page))
            .unwrap_or(false)
    }

    /// Takes the advisory lock (and keeps trying), then recovers rooms that were behind when the
    /// previous owner stopped. The returned task runs for the life of the process.
    pub fn spawn_owner(self: &Arc<Self>, database_url: String) -> tokio::task::JoinHandle<()> {
        let hub = self.clone();
        tokio::spawn(async move {
            while !hub.shutting_down.load(Ordering::SeqCst) {
                match PgConnection::connect(&database_url).await {
                    Ok(mut conn) => {
                        let got: Result<bool, _> =
                            sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
                                .bind(hub.lock_key)
                                .fetch_one(&mut conn)
                                .await;
                        if matches!(got, Ok(true)) {
                            hub.holds_lock.store(true, Ordering::SeqCst);
                            tracing::info!("doc co-editing: this instance owns the rooms");
                            hub.recover().await;
                            // The lock lives as long as this connection; losing it ends ownership.
                            loop {
                                if hub.shutting_down.load(Ordering::SeqCst) {
                                    let _ = sqlx::query("SELECT pg_advisory_unlock($1)")
                                        .bind(hub.lock_key)
                                        .execute(&mut conn)
                                        .await;
                                    return;
                                }
                                tokio::time::sleep(Duration::from_secs(2)).await;
                                if conn.ping().await.is_err() {
                                    break;
                                }
                            }
                            hub.holds_lock.store(false, Ordering::SeqCst);
                            hub.close_all(close::TRY_LATER, "co-editing ownership lost")
                                .await;
                            tracing::warn!("doc co-editing: the lock connection was lost");
                        }
                    }
                    Err(error) => tracing::warn!(?error, "doc co-editing: lock connection failed"),
                }
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        })
    }

    /// Opens the rooms whose last materialization is behind their log; each one materializes at once
    /// and unloads after the idle period (crash recovery, T31).
    async fn recover(self: &Arc<Self>) {
        match store::unmaterialized(&self.pool).await {
            Ok(pages) => {
                for page in pages {
                    if let Err(error) = self.ensure_room(page).await {
                        tracing::warn!(%page, ?error, "doc co-editing: recovery of a page failed");
                    }
                }
            }
            Err(error) => tracing::warn!(?error, "doc co-editing: recovery query failed"),
        }
    }

    async fn ensure_room(self: &Arc<Self>, page: Uuid) -> Result<RoomHandle, JoinError> {
        if !self.holds_lock() {
            return Err(JoinError::Unavailable);
        }
        if let Some(handle) = self.rooms.lock().ok().and_then(|r| r.get(&page).cloned())
            && !handle.tx.is_closed()
        {
            return Ok(handle);
        }
        {
            let rooms = self.rooms.lock().map_err(|_| JoinError::Busy)?;
            if rooms.len() as u64 >= self.config.collab_max_rooms {
                return Err(JoinError::TooManyRooms);
            }
        }
        let loaded = match store::load_or_seed(&self.pool, page).await {
            Ok(loaded) => loaded,
            Err(store::LoadError::Gone) => return Err(JoinError::NoAccess),
            Err(store::LoadError::Db(error)) => {
                tracing::warn!(%page, ?error, "doc co-editing: room could not be loaded");
                return Err(JoinError::Busy);
            }
        };
        let token = self.next_token.fetch_add(1, Ordering::SeqCst);
        let hub = self.clone();
        let spawned = room::spawn(
            self.pool.clone(),
            self.config.clone(),
            page,
            loaded,
            move || {
                if let Ok(mut rooms) = hub.rooms.lock()
                    && rooms.get(&page).is_some_and(|h| h.token == token)
                {
                    rooms.remove(&page);
                }
            },
        );
        let handle = RoomHandle {
            token,
            tx: spawned.tx,
        };
        let mut rooms = self.rooms.lock().map_err(|_| JoinError::Busy)?;
        // Two openers may have raced; the first one wins and the second room is told to stop.
        if let Some(existing) = rooms.get(&page)
            && !existing.tx.is_closed()
        {
            let (done, _) = oneshot::channel();
            let _ = handle.tx.send(RoomCmd::Shutdown { done });
            return Ok(existing.clone());
        }
        rooms.insert(page, handle.clone());
        Ok(handle)
    }

    /// Adds a connection to the page's room. The caller has already answered the HTTP-level
    /// authorization; the room asks PostgreSQL again before it lets the connection in.
    pub async fn join(
        self: &Arc<Self>,
        page: Uuid,
        user: UserInfo,
    ) -> Result<CollabConnection, JoinError> {
        {
            let mut connections = self.connections.lock().map_err(|_| JoinError::Busy)?;
            let count = connections.entry((user.id, page)).or_insert(0);
            if *count as u64 >= self.config.collab_max_connections_per_user_doc {
                return Err(JoinError::TooManyConnections);
            }
            *count += 1;
        }
        let result = self.join_counted(page, user.clone()).await;
        if result.is_err() {
            self.release(user.id, page);
        }
        result
    }

    async fn join_counted(
        self: &Arc<Self>,
        page: Uuid,
        user: UserInfo,
    ) -> Result<CollabConnection, JoinError> {
        for _ in 0..2 {
            let handle = self.ensure_room(page).await?;
            let conn = self.next_conn.fetch_add(1, Ordering::SeqCst);
            let (tx, rx) = mpsc::unbounded_channel();
            let (reply, answer) = oneshot::channel();
            if handle
                .tx
                .send(RoomCmd::Join {
                    conn,
                    user: user.clone(),
                    tx,
                    reply,
                })
                .is_err()
            {
                continue; // the room stopped between lookup and join; open a new one
            }
            match answer.await {
                Ok(Ok(info)) => {
                    return Ok(CollabConnection {
                        id: conn,
                        epoch: info.epoch,
                        rx,
                        sender: handle.tx,
                        hub: self.clone(),
                        user: user.id,
                        page,
                    });
                }
                Ok(Err(JoinDenied::NoAccess)) => return Err(JoinError::NoAccess),
                Ok(Err(JoinDenied::Busy)) => return Err(JoinError::Busy),
                Err(_) => continue,
            }
        }
        Err(JoinError::Busy)
    }

    fn release(&self, user: Uuid, page: Uuid) {
        if let Ok(mut connections) = self.connections.lock()
            && let Some(count) = connections.get_mut(&(user, page))
        {
            *count = count.saturating_sub(1);
            if *count == 0 {
                connections.remove(&(user, page));
            }
        }
    }

    fn handle_of(&self, page: Uuid) -> Option<RoomHandle> {
        self.rooms.lock().ok().and_then(|r| r.get(&page).cloned())
    }

    /// Permission, visibility or participants of a page changed: every connection is checked
    /// against PostgreSQL now (section F.9).
    pub async fn recheck(&self, page: Uuid) {
        if let Some(handle) = self.handle_of(page) {
            let _ = handle.tx.send(RoomCmd::Recheck);
        }
    }

    /// The page was deleted: connections get 4404.
    pub async fn page_deleted(&self, page: Uuid) {
        if let Some(handle) = self.handle_of(page) {
            let _ = handle.tx.send(RoomCmd::Deleted);
        }
    }

    pub fn page_meta_changed(&self, page: Uuid, title: &str) {
        if let Some(handle) = self.handle_of(page) {
            let _ = handle.tx.send(RoomCmd::Meta {
                title: title.to_owned(),
            });
        }
    }

    pub fn comments_changed(&self, page: Uuid) {
        if let Some(handle) = self.handle_of(page) {
            let _ = handle.tx.send(RoomCmd::CommentsChanged);
        }
    }

    /// State of a loaded room (tests and diagnostics).
    pub async fn stats(&self, page: Uuid) -> Option<Value> {
        let handle = self.handle_of(page)?;
        let (reply, answer) = oneshot::channel();
        handle.tx.send(RoomCmd::Stats { reply }).ok()?;
        answer.await.ok()
    }

    async fn close_all(&self, code: u16, reason: &'static str) {
        let handles: Vec<RoomHandle> = self
            .rooms
            .lock()
            .map(|r| r.values().cloned().collect())
            .unwrap_or_default();
        let _ = (code, reason);
        for handle in handles {
            let (done, wait) = oneshot::channel();
            if handle.tx.send(RoomCmd::Shutdown { done }).is_ok() {
                let _ = tokio::time::timeout(Duration::from_secs(10), wait).await;
            }
        }
    }

    /// Test hook: every room stops at once without storing anything, as a crash would.
    pub fn crash_for_tests(&self) {
        self.shutting_down.store(true, Ordering::SeqCst);
        let handles: Vec<RoomHandle> = self
            .rooms
            .lock()
            .map(|r| r.values().cloned().collect())
            .unwrap_or_default();
        for handle in handles {
            let _ = handle.tx.send(RoomCmd::Crash);
        }
    }

    /// Graceful stop: every room stores what it has, materializes, and says goodbye (1001).
    pub async fn shutdown(&self) {
        self.shutting_down.store(true, Ordering::SeqCst);
        self.close_all(close::GOING_AWAY, "server shutting down")
            .await;
    }

    /// Whether a person may open the co-editing socket of a page (the HTTP-level check before the
    /// upgrade; the room checks again). `None` is "not found".
    pub async fn may_open(pool: &PgPool, user: Uuid, page: Uuid) -> Result<bool, crate::CoreError> {
        let mut conn = pool.acquire().await?;
        let rights = visibility::doc_rights(&mut conn, user, &[page]).await?;
        Ok(rights.get(&page).is_some_and(|r| r.read && r.edit))
    }
}

/// `{"type":"hello",...}` is built by the room; this documents the frames the server sends as text.
pub fn control_frames() -> Value {
    json!([
        "hello",
        "meta",
        "comments_changed",
        "materialized",
        "revoked"
    ])
}
