//! One room: the Yrs document of one page, its connected editors, and its schedule.
//!
//! A room is a task that owns everything and takes its work from one channel, so no lock protects
//! the document and every decision (apply, persist, materialize, revoke) is made in a single order.
//! The comments below say where each guarantee is kept.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;
use uuid::Uuid;
use yrs::sync::{AwarenessUpdate, Message, SyncMessage};
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{ClientID, Doc, ReadTxn, Transact, Update};

use super::store::{self, AppendError, Loaded};
use super::{Outbound, UserInfo, close};
use crate::docs::materialize::{self, Applied, Refused};
use crate::docs::yjs;
use crate::knowledge::KnowledgeConfig;
use crate::visibility;

pub(super) enum RoomCmd {
    Join {
        conn: u64,
        user: UserInfo,
        tx: mpsc::UnboundedSender<Outbound>,
        reply: oneshot::Sender<Result<JoinInfo, JoinDenied>>,
    },
    Leave {
        conn: u64,
    },
    Frame {
        conn: u64,
        data: Vec<u8>,
    },
    /// Re-evaluate every connection against PostgreSQL now.
    Recheck,
    /// The page was deleted.
    Deleted,
    Meta {
        title: String,
    },
    CommentsChanged,
    /// Flush, close the clients (1001) and stop.
    Shutdown {
        done: oneshot::Sender<()>,
    },
    Stats {
        reply: oneshot::Sender<Value>,
    },
    /// Test hook: stop without storing anything.
    Crash,
}

#[derive(Debug)]
pub struct JoinInfo {
    pub epoch: Uuid,
}

#[derive(Debug)]
pub enum JoinDenied {
    NoAccess,
    Busy,
}

struct Client {
    user: UserInfo,
    tx: mpsc::UnboundedSender<Outbound>,
    last_auth: Instant,
    /// Awareness client ids this connection speaks for. A connection may not set another's.
    owned: HashSet<ClientID>,
}

struct AwarenessEntry {
    clock: u32,
    json: Option<Value>,
    owner: u64,
}

const MAX_AWARENESS_IDS_PER_CONN: usize = 4;
const MAX_AWARENESS_BYTES: usize = 4096;
const PERSIST_GIVE_UP: Duration = Duration::from_secs(30);

const COLORS: &[&str] = &[
    "#d9480f", "#1971c2", "#2f9e44", "#9c36b5", "#c2255c", "#0c8599", "#e8590c", "#5f3dc4",
];

pub fn color_of(user: Uuid) -> &'static str {
    COLORS[(user.as_bytes()[0] as usize + user.as_bytes()[15] as usize) % COLORS.len()]
}

struct Room {
    pool: PgPool,
    config: Arc<KnowledgeConfig>,
    page_id: Uuid,
    doc: Doc,
    epoch: Uuid,
    /// Highest log position that is in the database.
    persisted_seq: i64,
    snapshot_seq: i64,
    materialized_seq: i64,
    state_bytes: u64,
    pending: Vec<(Option<Uuid>, Vec<u8>)>,
    persist_failing_since: Option<Instant>,
    clients: HashMap<u64, Client>,
    awareness: HashMap<ClientID, AwarenessEntry>,
    editors: BTreeSet<Uuid>,
    /// Updates exist that `doc_pages.content` does not show yet.
    dirty: bool,
    first_dirty_at: Option<Instant>,
    last_update_at: Option<Instant>,
    /// The materializer refused the current state; nothing is retried until the state changes.
    invalid: bool,
    persist_retry_at: Option<Instant>,
    materialize_retry_at: Option<Instant>,
    retry_delay: Duration,
    empty_since: Option<Instant>,
    next_sweep: Instant,
    last_revision: Option<i64>,
    degraded: bool,
    /// The lineage in the database is not this room's any more; the room must stop.
    epoch_lost: bool,
    /// The page was deleted while the room was open.
    gone: bool,
}

pub(super) struct Spawned {
    pub tx: mpsc::UnboundedSender<RoomCmd>,
}

/// Starts the task. `on_exit` is called once when the room has stopped.
pub(super) fn spawn(
    pool: PgPool,
    config: Arc<KnowledgeConfig>,
    page_id: Uuid,
    loaded: Loaded,
    on_exit: impl FnOnce() + Send + 'static,
) -> Spawned {
    let (tx, rx) = mpsc::unbounded_channel();
    let now = Instant::now();
    let behind = loaded.materialized_seq < loaded.last_seq;
    let room = Room {
        pool,
        config: config.clone(),
        page_id,
        doc: loaded.doc,
        epoch: loaded.epoch,
        persisted_seq: loaded.last_seq,
        snapshot_seq: loaded.snapshot_seq,
        materialized_seq: loaded.materialized_seq,
        state_bytes: loaded.state_bytes,
        pending: Vec::new(),
        persist_failing_since: None,
        clients: HashMap::new(),
        awareness: HashMap::new(),
        editors: BTreeSet::new(),
        // A log ahead of the last materialization means a crash (or a failed run) came between.
        dirty: behind,
        first_dirty_at: behind.then_some(now),
        last_update_at: behind.then_some(now),
        invalid: false,
        persist_retry_at: None,
        materialize_retry_at: None,
        retry_delay: Duration::from_secs(1),
        empty_since: Some(now),
        next_sweep: now + Duration::from_secs(config.collab_auth_ttl_seconds.max(1)),
        last_revision: None,
        degraded: false,
        epoch_lost: false,
        gone: false,
    };
    tokio::spawn(async move {
        room.run(rx).await;
        on_exit();
    });
    Spawned { tx }
}

fn text(value: Value) -> Outbound {
    Outbound::Text(value.to_string())
}

impl Room {
    async fn run(mut self, mut rx: mpsc::UnboundedReceiver<RoomCmd>) {
        loop {
            let deadline = self.next_deadline();
            tokio::select! {
                cmd = rx.recv() => {
                    let Some(cmd) = cmd else { break };
                    match cmd {
                        RoomCmd::Crash => return,
                        RoomCmd::Shutdown { done } => {
                            self.finish(close::GOING_AWAY, "server shutting down", true).await;
                            let _ = done.send(());
                            return;
                        }
                        other => {
                            if !self.handle(other).await || self.gone || self.epoch_lost {
                                return;
                            }
                        }
                    }
                }
                _ = tokio::time::sleep_until(deadline) => {
                    if !self.tick().await {
                        return;
                    }
                }
            }
        }
        self.finish(close::GOING_AWAY, "room closed", true).await;
    }

    fn next_deadline(&self) -> Instant {
        let cfg = &self.config;
        let mut at = self.next_sweep;
        let mut consider = |t: Instant| {
            if t < at {
                at = t;
            }
        };
        if !self.pending.is_empty() {
            consider(self.persist_due());
        }
        if self.dirty && !self.invalid {
            consider(self.materialize_due());
        }
        if let Some(since) = self.empty_since {
            consider(since + Duration::from_secs(cfg.collab_room_idle_seconds));
        }
        at
    }

    fn persist_due(&self) -> Instant {
        let base = self.last_update_at.unwrap_or_else(Instant::now);
        let due = base + Duration::from_millis(self.config.collab_persist_interval_ms);
        match self.persist_retry_at {
            Some(retry) if retry > due => retry,
            _ => due,
        }
    }

    fn materialize_due(&self) -> Instant {
        let now = Instant::now();
        let debounce = Duration::from_millis(self.config.materialize_debounce_ms);
        let max_wait = Duration::from_millis(self.config.materialize_max_wait_ms);
        let by_quiet = self.last_update_at.unwrap_or(now) + debounce;
        let by_age = self.first_dirty_at.unwrap_or(now) + max_wait;
        let due = by_quiet.min(by_age);
        match self.materialize_retry_at {
            Some(retry) if retry > due => retry,
            _ => due,
        }
    }

    // -----------------------------------------------------------------------------------------
    // Commands
    // -----------------------------------------------------------------------------------------

    /// Returns false when the room has stopped.
    async fn handle(&mut self, cmd: RoomCmd) -> bool {
        match cmd {
            RoomCmd::Join {
                conn,
                user,
                tx,
                reply,
            } => {
                let result = self.join(conn, user, tx).await;
                let _ = reply.send(result);
            }
            RoomCmd::Leave { conn } => self.leave(conn).await,
            RoomCmd::Frame { conn, data } => self.frame(conn, data).await,
            RoomCmd::Recheck => self.sweep().await,
            RoomCmd::Deleted => {
                self.finish(close::GONE, "page deleted", false).await;
                return false;
            }
            RoomCmd::Meta { title } => {
                self.broadcast(None, text(json!({"type": "meta", "title": title})));
            }
            RoomCmd::CommentsChanged => {
                self.broadcast(None, text(json!({"type": "comments_changed"})));
            }
            RoomCmd::Stats { reply } => {
                let _ = reply.send(json!({
                    "clients": self.clients.len(),
                    "dirty": self.dirty,
                    "invalid": self.invalid,
                    "epoch": self.epoch,
                    "persistedSeq": self.persisted_seq,
                    "materializedSeq": self.materialized_seq,
                    "pending": self.pending.len(),
                    "degraded": self.degraded,
                }));
            }
            RoomCmd::Shutdown { .. } | RoomCmd::Crash => unreachable!("handled by run"),
        }
        true
    }

    async fn join(
        &mut self,
        conn: u64,
        user: UserInfo,
        tx: mpsc::UnboundedSender<Outbound>,
    ) -> Result<JoinInfo, JoinDenied> {
        if self.degraded {
            return Err(JoinDenied::Busy);
        }
        match self.may_edit(user.id).await {
            Ok(true) => {}
            Ok(false) => return Err(JoinDenied::NoAccess),
            Err(()) => return Err(JoinDenied::Busy),
        }
        let state_vector = self.doc.transact().state_vector();
        let _ = tx.send(text(json!({
            "type": "hello", "v": 1, "epoch": self.epoch, "role": "EDITOR",
            "maxUpdateBytes": self.config.collab_max_update_bytes,
            "user": {"userId": user.id, "name": user.name, "color": color_of(user.id)},
        })));
        let _ = tx.send(Outbound::Binary(
            Message::Sync(SyncMessage::SyncStep1(state_vector)).encode_v1(),
        ));
        if let Some(update) = self.awareness_snapshot() {
            let _ = tx.send(Outbound::Binary(Message::Awareness(update).encode_v1()));
        }
        self.clients.insert(
            conn,
            Client {
                user,
                tx,
                last_auth: Instant::now(),
                owned: HashSet::new(),
            },
        );
        self.empty_since = None;
        Ok(JoinInfo { epoch: self.epoch })
    }

    async fn leave(&mut self, conn: u64) {
        self.drop_client(conn, None);
        if self.clients.is_empty() {
            self.empty_since = Some(Instant::now());
            // The last editor left: what they wrote reaches PostgreSQL now, not after the quiet period.
            if self.dirty && !self.invalid {
                self.materialize().await;
            }
        }
    }

    /// Removes a connection and tells the others its cursors are gone.
    fn drop_client(&mut self, conn: u64, close: Option<(u16, &'static str)>) {
        let Some(client) = self.clients.remove(&conn) else {
            return;
        };
        if let Some((code, reason)) = close {
            let _ = client.tx.send(Outbound::Close(code, reason));
        }
        let mut removed = AwarenessUpdate {
            clients: HashMap::new(),
        };
        for id in client.owned {
            if let Some(entry) = self.awareness.remove(&id) {
                removed.clients.insert(
                    id,
                    yrs::sync::awareness::AwarenessUpdateEntry {
                        clock: entry.clock.saturating_add(1),
                        json: "null".into(),
                    },
                );
            }
        }
        if !removed.clients.is_empty() {
            self.broadcast(
                None,
                Outbound::Binary(Message::Awareness(removed).encode_v1()),
            );
        }
    }

    fn revoke(&mut self, conn: u64) {
        if let Some(client) = self.clients.get(&conn) {
            let _ = client.tx.send(text(json!({"type": "revoked"})));
        }
        self.drop_client(conn, Some((close::REVOKED, "access revoked")));
    }

    fn broadcast(&self, except: Option<u64>, frame: Outbound) {
        for (id, client) in &self.clients {
            if Some(*id) != except {
                let _ = client.tx.send(clone_frame(&frame));
            }
        }
    }

    // -----------------------------------------------------------------------------------------
    // Authorization (section F.4, F.9)
    // -----------------------------------------------------------------------------------------

    /// Whether the person may edit the page now, from PostgreSQL. `Err(())` is a database failure.
    async fn may_edit(&self, user: Uuid) -> Result<bool, ()> {
        let mut conn = self.pool.acquire().await.map_err(|_| ())?;
        let rights = visibility::doc_rights(&mut conn, user, &[self.page_id])
            .await
            .map_err(|_| ())?;
        Ok(rights.get(&self.page_id).is_some_and(|r| r.edit))
    }

    /// Checks one connection when its last check is older than the configured age. Returns whether
    /// it may go on; on failure the connection has been closed.
    async fn authorize(&mut self, conn: u64) -> bool {
        let ttl = Duration::from_secs(self.config.collab_auth_ttl_seconds.max(1));
        let Some(client) = self.clients.get(&conn) else {
            return false;
        };
        if client.last_auth.elapsed() < ttl {
            return true;
        }
        let user = client.user.id;
        match self.may_edit(user).await {
            Ok(true) => {
                if let Some(client) = self.clients.get_mut(&conn) {
                    client.last_auth = Instant::now();
                }
                true
            }
            Ok(false) => {
                self.revoke(conn);
                false
            }
            Err(()) => {
                self.drop_client(conn, Some((close::TRY_LATER, "try again")));
                false
            }
        }
    }

    /// Re-evaluates every connection (revocation hooks and the periodic sweep).
    async fn sweep(&mut self) {
        let users: HashSet<Uuid> = self.clients.values().map(|c| c.user.id).collect();
        let mut denied: HashSet<Uuid> = HashSet::new();
        let mut failed = false;
        for user in users {
            match self.may_edit(user).await {
                Ok(true) => {}
                Ok(false) => {
                    denied.insert(user);
                }
                Err(()) => failed = true,
            }
        }
        let now = Instant::now();
        let victims: Vec<u64> = self
            .clients
            .iter()
            .filter(|(_, c)| denied.contains(&c.user.id))
            .map(|(id, _)| *id)
            .collect();
        for conn in victims {
            self.revoke(conn);
        }
        if !failed {
            for client in self.clients.values_mut() {
                client.last_auth = now;
            }
        }
        if self.clients.is_empty() && self.empty_since.is_none() {
            self.empty_since = Some(now);
        }
    }

    // -----------------------------------------------------------------------------------------
    // Frames
    // -----------------------------------------------------------------------------------------

    async fn frame(&mut self, conn: u64, data: Vec<u8>) {
        if !self.clients.contains_key(&conn) {
            return;
        }
        if data.len() as u64 > self.config.collab_max_update_bytes + 1024 {
            self.drop_client(conn, Some((close::TOO_LARGE, "update too large")));
            return;
        }
        let mut messages = Vec::new();
        let mut decoder = yrs::updates::decoder::DecoderV1::from(data.as_slice());
        let mut reader = yrs::sync::MessageReader::new(&mut decoder);
        for message in &mut reader {
            match message {
                Ok(m) => messages.push(m),
                Err(_) => {
                    self.drop_client(conn, Some((close::BAD_MESSAGE, "malformed message")));
                    return;
                }
            }
        }
        for message in messages {
            if !self.clients.contains_key(&conn) {
                return;
            }
            match message {
                Message::Sync(SyncMessage::SyncStep1(sv)) => {
                    let update = self.doc.transact().encode_state_as_update_v1(&sv);
                    self.send(
                        conn,
                        Outbound::Binary(Message::Sync(SyncMessage::SyncStep2(update)).encode_v1()),
                    );
                }
                Message::Sync(SyncMessage::SyncStep2(bytes))
                | Message::Sync(SyncMessage::Update(bytes)) => {
                    self.apply_update(conn, bytes).await;
                }
                Message::Awareness(update) => self.on_awareness(conn, update),
                Message::AwarenessQuery => {
                    if let Some(update) = self.awareness_snapshot() {
                        self.send(
                            conn,
                            Outbound::Binary(Message::Awareness(update).encode_v1()),
                        );
                    }
                }
                Message::Auth(_) | Message::Custom(..) => {}
            }
        }
    }

    fn send(&self, conn: u64, frame: Outbound) {
        if let Some(client) = self.clients.get(&conn) {
            let _ = client.tx.send(frame);
        }
    }

    async fn apply_update(&mut self, conn: u64, bytes: Vec<u8>) {
        if self.degraded {
            self.drop_client(conn, Some((close::TRY_LATER, "try again")));
            return;
        }
        // Permission is decided before the update touches the document (section F.4).
        if !self.authorize(conn).await {
            return;
        }
        if bytes.len() as u64 > self.config.collab_max_update_bytes
            || self.state_bytes + bytes.len() as u64 > self.config.collab_max_state_bytes
        {
            self.drop_client(conn, Some((close::TOO_LARGE, "update too large")));
            return;
        }
        let decoded = match Update::decode_v1(&bytes) {
            Ok(update) => update,
            Err(_) => {
                self.drop_client(conn, Some((close::BAD_MESSAGE, "malformed update")));
                return;
            }
        };
        let doc = self.doc.clone();
        let applied = catch_unwind(AssertUnwindSafe(|| {
            doc.transact_mut().apply_update(decoded)
        }));
        match applied {
            Ok(Ok(())) => {}
            Ok(Err(_)) | Err(_) => {
                self.drop_client(
                    conn,
                    Some((close::BAD_MESSAGE, "update could not be applied")),
                );
                return;
            }
        }
        let actor = self.clients.get(&conn).map(|c| c.user.id);
        if let Some(actor) = actor {
            self.editors.insert(actor);
        }
        self.state_bytes += bytes.len() as u64;
        self.pending.push((actor, bytes.clone()));
        let now = Instant::now();
        self.last_update_at = Some(now);
        if !self.dirty {
            self.dirty = true;
            self.first_dirty_at = Some(now);
        }
        // New state: a refusal of the previous state says nothing about this one.
        self.invalid = false;
        self.broadcast(
            Some(conn),
            Outbound::Binary(Message::Sync(SyncMessage::Update(bytes)).encode_v1()),
        );
    }

    // -----------------------------------------------------------------------------------------
    // Awareness (cursors); never persisted
    // -----------------------------------------------------------------------------------------

    fn awareness_snapshot(&self) -> Option<AwarenessUpdate> {
        let clients: HashMap<_, _> = self
            .awareness
            .iter()
            .filter_map(|(id, e)| {
                e.json.as_ref().map(|state| {
                    (
                        *id,
                        yrs::sync::awareness::AwarenessUpdateEntry {
                            clock: e.clock,
                            json: state.to_string().into(),
                        },
                    )
                })
            })
            .collect();
        (!clients.is_empty()).then_some(AwarenessUpdate { clients })
    }

    /// A client sends its cursor; the server decides who it is. Only a `cursor` payload is kept and
    /// `user` is always the authenticated person, never what the client wrote.
    fn on_awareness(&mut self, conn: u64, update: AwarenessUpdate) {
        let Some(client) = self.clients.get(&conn) else {
            return;
        };
        let identity = json!({
            "userId": client.user.id, "name": client.user.name, "color": color_of(client.user.id),
        });
        let mut forwarded = AwarenessUpdate {
            clients: HashMap::new(),
        };
        for (id, entry) in update.clients {
            let owned_here = self
                .clients
                .get(&conn)
                .is_some_and(|c| c.owned.contains(&id));
            let taken_elsewhere = self.awareness.get(&id).is_some_and(|e| e.owner != conn);
            if taken_elsewhere {
                continue;
            }
            if !owned_here {
                let count = self.clients.get(&conn).map_or(0, |c| c.owned.len());
                if count >= MAX_AWARENESS_IDS_PER_CONN {
                    continue;
                }
            }
            if entry.json.len() > MAX_AWARENESS_BYTES {
                continue;
            }
            let state: Option<Value> = match serde_json::from_str::<Value>(&entry.json) {
                Ok(Value::Null) => None,
                Ok(Value::Object(map)) => {
                    let mut clean = serde_json::Map::new();
                    if let Some(cursor) = map.get("cursor") {
                        clean.insert("cursor".into(), cursor.clone());
                    }
                    clean.insert("user".into(), identity.clone());
                    Some(Value::Object(clean))
                }
                _ => continue,
            };
            if let Some(client) = self.clients.get_mut(&conn) {
                client.owned.insert(id);
            }
            let clock = entry.clock;
            if self.awareness.get(&id).is_some_and(|e| e.clock > clock) {
                continue;
            }
            let json_text: Arc<str> = match &state {
                Some(v) => v.to_string().into(),
                None => "null".into(),
            };
            self.awareness.insert(
                id,
                AwarenessEntry {
                    clock,
                    json: state.clone(),
                    owner: conn,
                },
            );
            forwarded.clients.insert(
                id,
                yrs::sync::awareness::AwarenessUpdateEntry {
                    clock,
                    json: json_text,
                },
            );
            if state.is_none() {
                self.awareness.remove(&id);
            }
        }
        if !forwarded.clients.is_empty() {
            self.broadcast(
                Some(conn),
                Outbound::Binary(Message::Awareness(forwarded).encode_v1()),
            );
        }
    }

    // -----------------------------------------------------------------------------------------
    // Persisting and materializing (section F.6)
    // -----------------------------------------------------------------------------------------

    async fn tick(&mut self) -> bool {
        let now = Instant::now();
        if now >= self.next_sweep {
            self.next_sweep = now + Duration::from_secs(self.config.collab_auth_ttl_seconds.max(1));
            self.sweep().await;
            // An operator reset replaced the lineage: this room's state is no longer the page's.
            match store::current_epoch(&self.pool, self.page_id).await {
                Ok(Some(epoch)) if epoch == self.epoch => {}
                Ok(_) => {
                    self.finish(close::RESTART, "lineage was reset", false)
                        .await;
                    return false;
                }
                Err(_) => {}
            }
        }
        if !self.pending.is_empty() && now >= self.persist_due() {
            self.persist().await;
            if self.epoch_lost {
                return false;
            }
        }
        if self.dirty && !self.invalid && Instant::now() >= self.materialize_due() {
            self.materialize().await;
            if self.gone {
                return false;
            }
        }
        if let Some(since) = self.empty_since
            && now >= since + Duration::from_secs(self.config.collab_room_idle_seconds)
            && self.clients.is_empty()
        {
            self.finish(close::GOING_AWAY, "room idle", true).await;
            return false;
        }
        true
    }
}

fn clone_frame(frame: &Outbound) -> Outbound {
    match frame {
        Outbound::Binary(b) => Outbound::Binary(b.clone()),
        Outbound::Text(t) => Outbound::Text(t.clone()),
        Outbound::Close(code, reason) => Outbound::Close(*code, reason),
    }
}

impl Room {
    /// Appends the pending updates to the durable log. A crash can lose at most one persist
    /// interval; the connected clients still hold those updates and send them again when they
    /// reconnect, so the normal sync repairs it (section F.6).
    async fn persist(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let items = std::mem::take(&mut self.pending);
        match store::append(&self.pool, self.page_id, self.epoch, &items).await {
            Ok(last) => {
                self.persisted_seq = last;
                self.persist_failing_since = None;
                self.persist_retry_at = None;
                if (self.persisted_seq - self.snapshot_seq) as u64
                    >= self.config.collab_compact_updates
                {
                    self.compact().await;
                }
            }
            Err(AppendError::EpochChanged) => {
                self.epoch_lost = true;
                self.close_everyone(close::RESTART, "lineage was reset");
            }
            Err(AppendError::Db(error)) => {
                tracing::warn!(page = %self.page_id, ?error, "doc_collab_persist_failed");
                let mut back = items;
                back.append(&mut self.pending);
                self.pending = back;
                let since = *self.persist_failing_since.get_or_insert_with(Instant::now);
                self.persist_retry_at = Some(Instant::now() + Duration::from_secs(1));
                if since.elapsed() > PERSIST_GIVE_UP {
                    // Updates that cannot be kept are not accepted any more (section F.10).
                    self.degraded = true;
                    let conns: Vec<u64> = self.clients.keys().copied().collect();
                    for conn in conns {
                        self.drop_client(conn, Some((close::TRY_LATER, "storage unavailable")));
                    }
                }
            }
        }
    }

    async fn compact(&mut self) {
        let snapshot = yjs::encode_state(&self.doc);
        match store::compact(
            &self.pool,
            self.page_id,
            self.epoch,
            &snapshot,
            self.persisted_seq,
        )
        .await
        {
            Ok(true) => {
                self.snapshot_seq = self.persisted_seq;
                self.state_bytes = snapshot.len() as u64;
            }
            Ok(false) => {}
            Err(error) => tracing::warn!(page = %self.page_id, ?error, "doc_collab_compact_failed"),
        }
    }

    /// Turns the current Yjs state into `doc_pages.content` (the only path that writes it).
    async fn materialize(&mut self) {
        self.persist().await;
        if !self.pending.is_empty() || self.epoch_lost {
            self.schedule_materialize_retry();
            return;
        }
        let seq = self.persisted_seq;
        let max = usize::try_from(self.config.doc_max_content_bytes).unwrap_or(usize::MAX);
        let candidate = match yjs::materialize(&self.doc, max) {
            Ok((content, _)) => content,
            Err(error) => {
                self.refuse(&error.0).await;
                return;
            }
        };
        let editors: Vec<Uuid> = self.editors.iter().copied().collect();
        match materialize::apply(
            &self.pool,
            &self.config,
            self.page_id,
            &candidate,
            &editors,
            Some(self.epoch),
        )
        .await
        {
            Ok(Applied::Written { revision_no }) | Ok(Applied::Unchanged { revision_no }) => {
                if let Err(error) =
                    store::set_materialized(&self.pool, self.page_id, self.epoch, seq).await
                {
                    tracing::warn!(page = %self.page_id, ?error, "doc_collab_mark_failed");
                }
                self.materialized_seq = seq;
                self.dirty = false;
                self.first_dirty_at = None;
                self.editors.clear();
                self.materialize_retry_at = None;
                self.retry_delay = Duration::from_secs(1);
                self.last_revision = Some(revision_no);
                self.broadcast(
                    None,
                    text(json!({"type": "materialized", "revisionNo": revision_no})),
                );
            }
            Ok(Applied::Stale) => {
                self.epoch_lost = true;
                self.close_everyone(close::RESTART, "lineage was reset");
            }
            Ok(Applied::Skipped) => {
                self.gone = true;
                self.close_everyone(close::GONE, "page deleted");
            }
            Err(Refused::Invalid(reason)) => self.refuse(&reason).await,
            Err(Refused::Db(error)) => {
                tracing::warn!(page = %self.page_id, ?error, "doc_materialize_failed");
                self.schedule_materialize_retry();
            }
        }
    }

    /// The state cannot become content (structure outside the allowed set, or too large). The last
    /// good content stays in PostgreSQL; an operator decides (`doc-collab-reset`). Nothing is
    /// retried until the state changes.
    async fn refuse(&mut self, reason: &str) {
        tracing::warn!(page = %self.page_id, reason, "doc_materialize_refused");
        if let Err(error) = store::set_invalid(&self.pool, self.page_id, self.epoch).await {
            tracing::warn!(page = %self.page_id, ?error, "doc_collab_mark_invalid_failed");
        }
        self.invalid = true;
    }

    fn schedule_materialize_retry(&mut self) {
        self.materialize_retry_at = Some(Instant::now() + self.retry_delay);
        self.retry_delay = (self.retry_delay * 2).min(Duration::from_secs(60));
    }

    /// Stops the room. With `flush` everything not yet stored is stored first.
    async fn finish(&mut self, code: u16, reason: &'static str, flush: bool) {
        if flush && !self.degraded && !self.epoch_lost && !self.gone {
            self.persist().await;
            if self.dirty && !self.invalid && !self.epoch_lost && !self.gone {
                self.materialize().await;
            }
            if self.pending.is_empty() && self.persisted_seq > self.snapshot_seq && !self.epoch_lost
            {
                self.compact().await;
            }
        }
        self.close_everyone(code, reason);
    }

    fn close_everyone(&mut self, code: u16, reason: &'static str) {
        let conns: Vec<u64> = self.clients.keys().copied().collect();
        for conn in conns {
            self.drop_client(conn, Some((code, reason)));
        }
    }
}
