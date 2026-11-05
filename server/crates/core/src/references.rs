//! "Referenced by" on a Ticket: the Docs that point at it.
//!
//! The source is the derived reference rows of Docs (`doc_page_targets`). An item is returned only when the caller may read the Ticket
//! and the source; an unreadable source is absent, with no count and no placeholder. Titles come from
//! PostgreSQL after the check. Conversations are not a source: no row links a Message to a Ticket.

use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::CoreError;
use crate::paging;
use crate::visibility;

#[derive(Clone)]
struct Candidate {
    id: Uuid,
    updated_at: DateTime<Utc>,
}

pub async fn for_ticket(
    pool: &PgPool,
    actor: Uuid,
    ticket_id: Uuid,
    cursor: Option<String>,
    limit: Option<i64>,
) -> Result<Value, CoreError> {
    incoming(pool, actor, "TICKET", ticket_id, cursor, limit).await
}

/// Docs that reference this Doc. Same rules as [`for_ticket`]: the caller must be able to read the
/// target, and a source they cannot read is absent. A page does not list itself.
pub async fn for_doc(
    pool: &PgPool,
    actor: Uuid,
    doc_id: Uuid,
    cursor: Option<String>,
    limit: Option<i64>,
) -> Result<Value, CoreError> {
    incoming(pool, actor, "DOC", doc_id, cursor, limit).await
}

async fn incoming(
    pool: &PgPool,
    actor: Uuid,
    target_kind: &str,
    target_id: Uuid,
    cursor: Option<String>,
    limit: Option<i64>,
) -> Result<Value, CoreError> {
    let limit = paging::require_limit(limit, 20, 50, "limit is 1 to 50")?;
    let mut conn = pool.acquire().await?;
    let readable = if target_kind == "DOC" {
        visibility::docs(&mut conn, actor, &[target_id]).await?
    } else {
        visibility::tickets(&mut conn, actor, &[target_id]).await?
    };
    if !readable.contains(&target_id) {
        return Err(CoreError::NotFound);
    }
    let workspace: Option<Uuid> = if target_kind == "DOC" {
        sqlx::query_scalar(
            "SELECT workspace_id FROM penfii.doc_pages WHERE id=$1 AND deleted_at IS NULL",
        )
        .bind(target_id)
        .fetch_optional(&mut *conn)
        .await?
    } else {
        sqlx::query_scalar("SELECT workspace_id FROM penfii.tickets WHERE id=$1")
            .bind(target_id)
            .fetch_optional(&mut *conn)
            .await?
    };
    let Some(workspace) = workspace else {
        return Err(CoreError::NotFound);
    };
    let mut after = cursor
        .as_deref()
        .map(paging::decode_cursor_hex)
        .transpose()?;
    let mut out: Vec<Value> = Vec::new();
    let mut last_seen: Option<(DateTime<Utc>, Uuid)> = None;
    let mut exhausted = false;
    // Candidates come newest first in batches; a batch may hold sources the caller cannot read, so
    // more batches are read until the page is full or the sources run out.
    while out.len() < limit as usize && !exhausted {
        let batch = sqlx::query(
            "SELECT kind, id, updated_at FROM (
               SELECT 'DOC' AS kind, p.id, p.updated_at FROM penfii.doc_page_targets t
                 JOIN penfii.doc_pages p ON p.id = t.page_id AND p.deleted_at IS NULL
                 WHERE t.target_kind=$6 AND t.target_id=$1 AND p.workspace_id=$2
                   AND ($6 <> 'DOC' OR p.id <> $1)
             ) s
             WHERE ($3::timestamptz IS NULL OR (updated_at, id) < ($3, $4))
             ORDER BY updated_at DESC, id DESC LIMIT $5",
        )
        .bind(target_id)
        .bind(workspace)
        .bind(after.map(|a| a.0))
        .bind(after.map(|a| a.1).unwrap_or(Uuid::max()))
        .bind(limit * 3 + 3)
        .bind(target_kind)
        .fetch_all(&mut *conn)
        .await?;
        if (batch.len() as i64) < limit * 3 + 3 {
            exhausted = true;
        }
        let candidates: Vec<Candidate> = batch
            .iter()
            .map(|r| {
                Ok(Candidate {
                    id: r.try_get("id")?,
                    updated_at: r.try_get("updated_at")?,
                })
            })
            .collect::<Result<_, sqlx::Error>>()?;
        if candidates.is_empty() {
            break;
        }
        let doc_ids: Vec<Uuid> = candidates.iter().map(|c| c.id).collect();
        let readable_docs = visibility::docs(&mut conn, actor, &doc_ids).await?;
        for candidate in &candidates {
            if out.len() >= limit as usize {
                break;
            }
            last_seen = Some((candidate.updated_at, candidate.id));
            after = last_seen;
            if !readable_docs.contains(&candidate.id) {
                continue;
            }
            {
                let row = sqlx::query(
                    "SELECT title, project_id, parent_id FROM penfii.doc_pages WHERE id=$1",
                )
                .bind(candidate.id)
                .fetch_one(&mut *conn)
                .await?;
                let path =
                    readable_parent_titles(&mut conn, actor, row.try_get("parent_id")?).await?;
                out.push(json!({
                    "sourceKind": "DOC", "sourceId": candidate.id, "title": row.try_get::<String, _>("title")?,
                    "context": {"projectId": row.try_get::<Option<Uuid>, _>("project_id")?, "parentTitlePath": path},
                    "updatedAt": candidate.updated_at.to_rfc3339(),
                }));
            }
        }
        if out.len() >= limit as usize {
            break;
        }
    }
    // More may follow when we stopped with a full page, or a batch was cut short by the page size.
    let next = if out.len() >= limit as usize {
        last_seen.map(|(at, id)| paging::encode_cursor_hex(at, id))
    } else {
        None
    };
    let next = match next {
        Some(cursor) if has_more(&mut conn, target_kind, target_id, workspace, &cursor).await? => {
            Some(cursor)
        }
        _ => None,
    };
    Ok(json!({"items": out, "nextCursor": next}))
}

/// Whether any candidate (readable or not) follows the cursor. A readable one may or may not exist;
/// the next request settles that, and may then be an empty page.
async fn has_more(
    conn: &mut sqlx::PgConnection,
    target_kind: &str,
    target_id: Uuid,
    workspace: Uuid,
    cursor: &str,
) -> Result<bool, CoreError> {
    let (at, id) = paging::decode_cursor_hex(cursor)?;
    let more: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM (
           SELECT p.id, p.updated_at FROM penfii.doc_page_targets t
             JOIN penfii.doc_pages p ON p.id = t.page_id AND p.deleted_at IS NULL
             WHERE t.target_kind=$5 AND t.target_id=$1 AND p.workspace_id=$2
               AND ($5 <> 'DOC' OR p.id <> $1)
         ) s WHERE (updated_at, id) < ($3, $4))",
    )
    .bind(target_id)
    .bind(workspace)
    .bind(at)
    .bind(id)
    .bind(target_kind)
    .fetch_one(&mut *conn)
    .await?;
    Ok(more)
}

/// Titles of the parents of a page, outermost first, up to the first one the caller cannot read:
/// the path never names a page the caller may not open.
async fn readable_parent_titles(
    conn: &mut sqlx::PgConnection,
    actor: Uuid,
    mut parent: Option<Uuid>,
) -> Result<Vec<String>, CoreError> {
    let mut titles = Vec::new();
    for _ in 0..16 {
        let Some(id) = parent else { break };
        if !visibility::docs(conn, actor, &[id]).await?.contains(&id) {
            break;
        }
        let row = sqlx::query("SELECT title, parent_id FROM penfii.doc_pages WHERE id=$1")
            .bind(id)
            .fetch_one(&mut *conn)
            .await?;
        titles.push(row.try_get::<String, _>("title")?);
        parent = row.try_get("parent_id")?;
    }
    titles.reverse();
    Ok(titles)
}
