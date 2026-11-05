//! Conversation and thread read cursors. Channel reads do not move thread reads.

use penfii_contracts::ReadState;
use serde_json::{Map, Value, json};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::Outcome;
use crate::error::CoreError;
use crate::platform;

pub struct UpdateRead {
    pub actor_id: Uuid,
    pub target_id: Uuid,
    pub idempotency_key: String,
    pub generation: i64,
    pub read_through_seq: i64,
    pub scope: String,
    pub observed: bool,
    pub raw: Value,
}

pub struct ManualUnread {
    pub actor_id: Uuid,
    pub target_id: Uuid,
    pub idempotency_key: String,
    pub from_seq: i64,
    pub raw: Value,
}

#[derive(Clone, Copy)]
enum Kind {
    Conversation,
    Thread,
}

impl Kind {
    fn table(self) -> &'static str {
        match self {
            Self::Conversation => "conversation_reads",
            Self::Thread => "thread_reads",
        }
    }

    fn column(self) -> &'static str {
        match self {
            Self::Conversation => "conversation_id",
            Self::Thread => "thread_root_id",
        }
    }

    fn read_kind(self) -> &'static str {
        match self {
            Self::Conversation => "read.update",
            Self::Thread => "thread.read.update",
        }
    }

    fn manual_kind(self) -> &'static str {
        match self {
            Self::Conversation => "read.manual_unread",
            Self::Thread => "thread.read.manual_unread",
        }
    }
}

#[derive(sqlx::FromRow)]
struct ReadRow {
    generation: i64,
    read_through_seq: i64,
    manual_unread_from_seq: Option<i64>,
}

pub async fn get_conversation_read(
    pool: &PgPool,
    user_id: Uuid,
    conversation_id: Uuid,
) -> Result<ReadState, CoreError> {
    let mut tx = pool.begin().await?;
    crate::access::require_read(&mut tx, user_id, conversation_id).await?;
    let state = ensure(&mut tx, Kind::Conversation, conversation_id, user_id).await?;
    tx.commit().await?;
    Ok(state)
}

pub async fn get_thread_read(
    pool: &PgPool,
    user_id: Uuid,
    root_id: Uuid,
) -> Result<ReadState, CoreError> {
    let mut tx = pool.begin().await?;
    super::require_thread_root(&mut tx, user_id, root_id).await?;
    let state = ensure(&mut tx, Kind::Thread, root_id, user_id).await?;
    tx.commit().await?;
    Ok(state)
}

pub async fn update_conversation_read(
    pool: &PgPool,
    cmd: UpdateRead,
) -> Result<Outcome<ReadState>, CoreError> {
    update_read(pool, Kind::Conversation, cmd).await
}

pub async fn update_thread_read(
    pool: &PgPool,
    cmd: UpdateRead,
) -> Result<Outcome<ReadState>, CoreError> {
    update_read(pool, Kind::Thread, cmd).await
}

pub async fn manual_conversation_unread(
    pool: &PgPool,
    cmd: ManualUnread,
) -> Result<Outcome<ReadState>, CoreError> {
    manual_unread(pool, Kind::Conversation, cmd).await
}

pub async fn manual_thread_unread(
    pool: &PgPool,
    cmd: ManualUnread,
) -> Result<Outcome<ReadState>, CoreError> {
    manual_unread(pool, Kind::Thread, cmd).await
}

pub(crate) async fn ensure_conversation_read(
    conn: &mut PgConnection,
    conversation_id: Uuid,
    user_id: Uuid,
) -> Result<ReadState, CoreError> {
    ensure(conn, Kind::Conversation, conversation_id, user_id).await
}

async fn update_read(
    pool: &PgPool,
    kind: Kind,
    cmd: UpdateRead,
) -> Result<Outcome<ReadState>, CoreError> {
    if cmd.read_through_seq < 0 {
        return Err(CoreError::validation("readThroughSeq is invalid"));
    }
    if cmd.scope != "auto" && cmd.scope != "explicit" {
        return Err(CoreError::validation("scope must be auto or explicit"));
    }
    let mut tx = pool.begin().await?;
    let workspace_id = resolve_target(&mut tx, kind, cmd.actor_id, cmd.target_id).await?;
    if !cmd.observed {
        return Err(CoreError::conflict(
            "read_not_observed",
            "read was not observed",
        ));
    }
    let current = ensure(&mut tx, kind, cmd.target_id, cmd.actor_id).await?;
    let actor = platform::actor_key(cmd.actor_id);
    let digest = platform::fingerprint(kind.read_kind(), &cmd.raw);
    if let Some(outcome) = platform::replay_outcome::<ReadState>(
        &mut tx,
        workspace_id,
        &actor,
        &cmd.idempotency_key,
        kind.read_kind(),
        &digest,
    )
    .await?
    {
        tx.commit().await?;
        return Ok(outcome);
    }
    let sql = if cmd.scope == "auto" {
        format!(
            "UPDATE penfii.{table}
             SET read_through_seq = GREATEST(read_through_seq, $3)
             WHERE {column} = $1 AND user_id = $2 AND generation = $4
               AND (manual_unread_from_seq IS NULL OR $3 < manual_unread_from_seq)
             RETURNING generation, read_through_seq, manual_unread_from_seq",
            table = kind.table(),
            column = kind.column()
        )
    } else {
        format!(
            "UPDATE penfii.{table}
             SET read_through_seq = GREATEST(read_through_seq, $3),
                 manual_unread_from_seq = NULL,
                 generation = generation + 1
             WHERE {column} = $1 AND user_id = $2 AND generation = $4
             RETURNING generation, read_through_seq, manual_unread_from_seq",
            table = kind.table(),
            column = kind.column()
        )
    };
    let updated = sqlx::query_as::<_, ReadRow>(&sql)
        .bind(cmd.target_id)
        .bind(cmd.actor_id)
        .bind(cmd.read_through_seq)
        .bind(cmd.generation)
        .fetch_optional(&mut *tx)
        .await?;
    let Some(row) = updated else {
        return Err(stale_read(&current));
    };
    let state = state_from(row);
    let body =
        serde_json::to_value(&state).map_err(|error| CoreError::Internal(error.to_string()))?;
    platform::complete(
        &mut tx,
        workspace_id,
        &actor,
        &cmd.idempotency_key,
        200,
        &body,
    )
    .await?;
    tx.commit().await?;
    Ok(Outcome {
        http_status: 200,
        value: state,
        changes: Vec::new(),
    })
}

async fn manual_unread(
    pool: &PgPool,
    kind: Kind,
    cmd: ManualUnread,
) -> Result<Outcome<ReadState>, CoreError> {
    if cmd.from_seq < 0 {
        return Err(CoreError::validation("fromSeq is invalid"));
    }
    let mut tx = pool.begin().await?;
    let workspace_id = resolve_target(&mut tx, kind, cmd.actor_id, cmd.target_id).await?;
    ensure(&mut tx, kind, cmd.target_id, cmd.actor_id).await?;
    let actor = platform::actor_key(cmd.actor_id);
    let digest = platform::fingerprint(kind.manual_kind(), &cmd.raw);
    if let Some(outcome) = platform::replay_outcome::<ReadState>(
        &mut tx,
        workspace_id,
        &actor,
        &cmd.idempotency_key,
        kind.manual_kind(),
        &digest,
    )
    .await?
    {
        tx.commit().await?;
        return Ok(outcome);
    }
    let sql = format!(
        "UPDATE penfii.{table}
         SET manual_unread_from_seq = $3, generation = generation + 1
         WHERE {column} = $1 AND user_id = $2
         RETURNING generation, read_through_seq, manual_unread_from_seq",
        table = kind.table(),
        column = kind.column()
    );
    let row = sqlx::query_as::<_, ReadRow>(&sql)
        .bind(cmd.target_id)
        .bind(cmd.actor_id)
        .bind(cmd.from_seq)
        .fetch_one(&mut *tx)
        .await?;
    let state = state_from(row);
    let body =
        serde_json::to_value(&state).map_err(|error| CoreError::Internal(error.to_string()))?;
    platform::complete(
        &mut tx,
        workspace_id,
        &actor,
        &cmd.idempotency_key,
        200,
        &body,
    )
    .await?;
    tx.commit().await?;
    Ok(Outcome {
        http_status: 200,
        value: state,
        changes: Vec::new(),
    })
}

async fn resolve_target(
    conn: &mut PgConnection,
    kind: Kind,
    user_id: Uuid,
    target_id: Uuid,
) -> Result<Uuid, CoreError> {
    let access = match kind {
        Kind::Conversation => crate::access::require_read(conn, user_id, target_id).await?,
        Kind::Thread => super::require_thread_root(conn, user_id, target_id).await?,
    };
    Ok(access.workspace_id)
}

async fn ensure(
    conn: &mut PgConnection,
    kind: Kind,
    target_id: Uuid,
    user_id: Uuid,
) -> Result<ReadState, CoreError> {
    let insert = format!(
        "INSERT INTO penfii.{table} ({column}, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        table = kind.table(),
        column = kind.column()
    );
    sqlx::query(&insert)
        .bind(target_id)
        .bind(user_id)
        .execute(&mut *conn)
        .await?;
    let select = format!(
        "SELECT generation, read_through_seq, manual_unread_from_seq
         FROM penfii.{table}
         WHERE {column} = $1 AND user_id = $2
         FOR UPDATE",
        table = kind.table(),
        column = kind.column()
    );
    let row = sqlx::query_as::<_, ReadRow>(&select)
        .bind(target_id)
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
    Ok(state_from(row))
}

fn state_from(row: ReadRow) -> ReadState {
    ReadState {
        generation: row.generation,
        read_through_seq: row.read_through_seq,
        manual_unread_from_seq: row.manual_unread_from_seq,
    }
}

fn stale_read(current: &ReadState) -> CoreError {
    let mut extra = Map::new();
    extra.insert("generation".into(), json!(current.generation));
    extra.insert(
        "manualUnread".into(),
        json!(current.manual_unread_from_seq.is_some()),
    );
    CoreError::conflict_extra("stale_read", "stale read", extra)
}
