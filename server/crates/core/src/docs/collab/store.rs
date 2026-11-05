//! The durable side of a room: lineage (epoch + snapshot) and the update log.
//!
//! Everything here is a recoverable working copy. `doc_pages` stays the truth for
//! the application; `store` only makes a restart or a crash cheap.

use sqlx::{PgPool, Row};
use uuid::Uuid;
use yrs::updates::decoder::Decode;
use yrs::{Doc, Transact, Update};

use crate::docs::yjs;
use crate::error::CoreError;

pub struct Loaded {
    pub doc: Doc,
    pub epoch: Uuid,
    pub snapshot_seq: i64,
    pub last_seq: i64,
    pub materialized_seq: i64,
    pub invalid: bool,
    /// Size of the encoded state, the base of the state-size limit.
    pub state_bytes: u64,
}

pub enum LoadError {
    /// The page does not exist or is deleted.
    Gone,
    Db(CoreError),
}

impl From<sqlx::Error> for LoadError {
    fn from(error: sqlx::Error) -> Self {
        LoadError::Db(error.into())
    }
}

impl From<CoreError> for LoadError {
    fn from(error: CoreError) -> Self {
        LoadError::Db(error)
    }
}

/// Opens the lineage of a page. With none, the server seeds one from `doc_pages.content` (clients
/// never seed content). Runs under the page's row lock so two openers cannot seed twice.
pub async fn load_or_seed(pool: &PgPool, page_id: Uuid) -> Result<Loaded, LoadError> {
    let mut tx = pool.begin().await?;
    let page = sqlx::query(
        "SELECT content FROM penfii.doc_pages WHERE id=$1 AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(page_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(LoadError::Gone)?;
    let state = sqlx::query(
        "SELECT epoch, snapshot, snapshot_seq, last_seq, materialized_seq, invalid_since IS NOT NULL AS invalid
         FROM penfii.doc_collab_state WHERE page_id=$1",
    )
    .bind(page_id)
    .fetch_optional(&mut *tx)
    .await?;
    let loaded = match state {
        Some(row) => {
            let snapshot: Vec<u8> = row.try_get("snapshot")?;
            let snapshot_seq: i64 = row.try_get("snapshot_seq")?;
            let doc = yjs::doc_from_update(&snapshot)
                .map_err(|e| LoadError::Db(CoreError::Internal(format!("collab snapshot: {e}"))))?;
            let updates = sqlx::query(
                "SELECT update FROM penfii.doc_collab_updates WHERE page_id=$1 AND seq > $2 ORDER BY seq",
            )
            .bind(page_id)
            .bind(snapshot_seq)
            .fetch_all(&mut *tx)
            .await?;
            for row in updates {
                let bytes: Vec<u8> = row.try_get("update")?;
                let update = Update::decode_v1(&bytes)
                    .map_err(|e| LoadError::Db(CoreError::Internal(format!("collab log: {e}"))))?;
                doc.transact_mut().apply_update(update).map_err(|e| {
                    LoadError::Db(CoreError::Internal(format!("collab log apply: {e}")))
                })?;
            }
            Loaded {
                state_bytes: yjs::encode_state(&doc).len() as u64,
                doc,
                epoch: row.try_get("epoch")?,
                snapshot_seq,
                last_seq: row.try_get("last_seq")?,
                materialized_seq: row.try_get("materialized_seq")?,
                invalid: row.try_get("invalid")?,
            }
        }
        None => {
            let content: serde_json::Value = page.try_get("content")?;
            let doc = yjs::seed_doc(&content)
                .map_err(|e| LoadError::Db(CoreError::Internal(format!("seed: {e}"))))?;
            let snapshot = yjs::encode_state(&doc);
            let epoch = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO penfii.doc_collab_state(page_id,epoch,snapshot) VALUES($1,$2,$3)",
            )
            .bind(page_id)
            .bind(epoch)
            .bind(&snapshot)
            .execute(&mut *tx)
            .await?;
            Loaded {
                state_bytes: snapshot.len() as u64,
                doc,
                epoch,
                snapshot_seq: 0,
                last_seq: 0,
                materialized_seq: 0,
                invalid: false,
            }
        }
    };
    tx.commit().await?;
    Ok(loaded)
}

pub enum AppendError {
    /// The lineage was replaced underneath the room (an operator reset it).
    EpochChanged,
    Db(CoreError),
}

/// Appends updates to the log and moves `last_seq`. Returns the new `last_seq`.
pub async fn append(
    pool: &PgPool,
    page_id: Uuid,
    epoch: Uuid,
    items: &[(Option<Uuid>, Vec<u8>)],
) -> Result<i64, AppendError> {
    let db = |e: sqlx::Error| AppendError::Db(e.into());
    let mut tx = pool.begin().await.map_err(db)?;
    let row = sqlx::query(
        "UPDATE penfii.doc_collab_state SET last_seq = last_seq + $3, updated_at = now()
         WHERE page_id=$1 AND epoch=$2 RETURNING last_seq",
    )
    .bind(page_id)
    .bind(epoch)
    .bind(items.len() as i64)
    .fetch_optional(&mut *tx)
    .await
    .map_err(db)?;
    let Some(row) = row else {
        return Err(AppendError::EpochChanged);
    };
    let last: i64 = row.try_get("last_seq").map_err(db)?;
    let first = last - items.len() as i64 + 1;
    for (i, (actor, bytes)) in items.iter().enumerate() {
        sqlx::query("INSERT INTO penfii.doc_collab_updates(page_id,seq,update,actor_id) VALUES($1,$2,$3,$4)")
            .bind(page_id)
            .bind(first + i as i64)
            .bind(bytes)
            .bind(actor)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
    }
    tx.commit().await.map_err(db)?;
    Ok(last)
}

/// Replaces the snapshot with the current state and drops the log it now contains. Only valid
/// when every applied update has been appended (`seq == last_seq`).
pub async fn compact(
    pool: &PgPool,
    page_id: Uuid,
    epoch: Uuid,
    snapshot: &[u8],
    seq: i64,
) -> Result<bool, CoreError> {
    let mut tx = pool.begin().await?;
    let done = sqlx::query(
        "UPDATE penfii.doc_collab_state SET snapshot=$3, snapshot_seq=$4, updated_at=now()
         WHERE page_id=$1 AND epoch=$2 AND last_seq=$4",
    )
    .bind(page_id)
    .bind(epoch)
    .bind(snapshot)
    .bind(seq)
    .execute(&mut *tx)
    .await?
    .rows_affected()
        > 0;
    if done {
        sqlx::query("DELETE FROM penfii.doc_collab_updates WHERE page_id=$1 AND seq <= $2")
            .bind(page_id)
            .bind(seq)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(done)
}

pub async fn set_materialized(
    pool: &PgPool,
    page_id: Uuid,
    epoch: Uuid,
    seq: i64,
) -> Result<(), CoreError> {
    sqlx::query(
        "UPDATE penfii.doc_collab_state SET materialized_seq=GREATEST(materialized_seq,$3), invalid_since=NULL, updated_at=now()
         WHERE page_id=$1 AND epoch=$2",
    )
    .bind(page_id)
    .bind(epoch)
    .bind(seq)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn set_invalid(pool: &PgPool, page_id: Uuid, epoch: Uuid) -> Result<(), CoreError> {
    sqlx::query(
        "UPDATE penfii.doc_collab_state SET invalid_since=COALESCE(invalid_since, now()), updated_at=now()
         WHERE page_id=$1 AND epoch=$2",
    )
    .bind(page_id)
    .bind(epoch)
    .execute(pool)
    .await?;
    Ok(())
}

/// The epoch the database holds now; `None` when the lineage (or the page) is gone.
pub async fn current_epoch(pool: &PgPool, page_id: Uuid) -> Result<Option<Uuid>, CoreError> {
    Ok(
        sqlx::query_scalar("SELECT epoch FROM penfii.doc_collab_state WHERE page_id=$1")
            .bind(page_id)
            .fetch_optional(pool)
            .await?,
    )
}

/// Pages whose last materialization is behind their log (a crash happened between).
pub async fn unmaterialized(pool: &PgPool) -> Result<Vec<Uuid>, CoreError> {
    Ok(sqlx::query_scalar(
        "SELECT s.page_id FROM penfii.doc_collab_state s
         JOIN penfii.doc_pages p ON p.id = s.page_id AND p.deleted_at IS NULL
         WHERE s.materialized_seq < s.last_seq",
    )
    .fetch_all(pool)
    .await?)
}

/// Operator reset: a new lineage seeded from the last good PostgreSQL content.
/// Returns the new epoch. Running rooms notice the changed epoch and disconnect their clients.
pub async fn reset(pool: &PgPool, page_id: Uuid) -> Result<Uuid, CoreError> {
    let mut tx = pool.begin().await?;
    let content: serde_json::Value = sqlx::query_scalar(
        "SELECT content FROM penfii.doc_pages WHERE id=$1 AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(page_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(CoreError::NotFound)?;
    let epoch = reseed(&mut tx, page_id, &content).await?;
    tx.commit().await?;
    Ok(epoch)
}

/// Starts a new lineage for a page from `content` inside the caller's transaction. A Save uses it so
/// the working copy the co-editing layer would open is the saved content, not what it held before.
/// Running rooms notice the changed epoch and disconnect their clients.
pub async fn reseed(
    conn: &mut sqlx::PgConnection,
    page_id: Uuid,
    content: &serde_json::Value,
) -> Result<Uuid, CoreError> {
    let doc = yjs::seed_doc(content).map_err(|e| CoreError::Internal(format!("seed: {e}")))?;
    let snapshot = yjs::encode_state(&doc);
    let epoch = Uuid::new_v4();
    sqlx::query("DELETE FROM penfii.doc_collab_updates WHERE page_id=$1")
        .bind(page_id)
        .execute(&mut *conn)
        .await?;
    sqlx::query(
        "INSERT INTO penfii.doc_collab_state(page_id,epoch,snapshot) VALUES($1,$2,$3)
         ON CONFLICT (page_id) DO UPDATE SET epoch=EXCLUDED.epoch, snapshot=EXCLUDED.snapshot,
           snapshot_seq=0, last_seq=0, materialized_seq=0, invalid_since=NULL, updated_at=now()",
    )
    .bind(page_id)
    .bind(epoch)
    .bind(&snapshot)
    .execute(&mut *conn)
    .await?;
    Ok(epoch)
}
