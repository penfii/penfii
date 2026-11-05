//! Drafts: the caller's unsent messages, kept on the server so they follow the person between
//! devices and can be listed in one place. A Draft belongs to one compose box (a channel, a thread
//! or a direct message). Saving over a newer version is refused so two tabs never silently
//! overwrite each other; sending (or emptying the box) removes the Draft.

use penfii_contracts::{Draft, DraftItem};
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool, Row};
use uuid::Uuid;

use crate::access;
use crate::error::CoreError;

const KINDS: &[&str] = &["CHANNEL_COMPOSE", "THREAD_COMPOSE", "DM_COMPOSE"];
const PREVIEW_CHARS: usize = 140;

/// Text of a rich text document in reading order, for a preview and for telling blank from filled.
fn plain_text(value: &Value, out: &mut String) {
    if let Some(text) = value.get("text").and_then(Value::as_str) {
        out.push_str(text);
    }
    if let Some(children) = value.get("content").and_then(Value::as_array) {
        for child in children {
            plain_text(child, out);
        }
        if matches!(
            value.get("type").and_then(Value::as_str),
            Some("paragraph" | "heading" | "codeBlock")
        ) {
            out.push(' ');
        }
    }
}

/// The conversation a compose box belongs to, and the thread root for a thread box. The caller
/// must be able to read the conversation, otherwise it looks like it does not exist.
async fn context(
    conn: &mut PgConnection,
    actor_id: Uuid,
    kind: &str,
    context_id: Uuid,
) -> Result<(Uuid, Uuid, Option<Uuid>, &'static str), CoreError> {
    if !KINDS.contains(&kind) {
        return Err(CoreError::validation("unknown draft context"));
    }
    let (conversation_id, thread) = if kind == "THREAD_COMPOSE" {
        let id: Uuid = sqlx::query_scalar(
            "SELECT conversation_id FROM penfii.messages WHERE id=$1 AND thread_root_id IS NULL",
        )
        .bind(context_id)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or(CoreError::NotFound)?;
        (id, Some(context_id))
    } else {
        (context_id, None)
    };
    let access = access::load_conversation(conn, actor_id, conversation_id)
        .await?
        .filter(|a| a.can_write())
        .ok_or(CoreError::NotFound)?;
    // A channel box and a direct-message box are the same thing to the client; the conversation
    // decides which of the two names is stored.
    let canonical = match (kind, access.kind.as_str()) {
        ("THREAD_COMPOSE", _) => "THREAD_COMPOSE",
        (_, "DM") => "DM_COMPOSE",
        _ => "CHANNEL_COMPOSE",
    };
    Ok((access.workspace_id, conversation_id, thread, canonical))
}

fn draft_from(row: &sqlx::postgres::PgRow) -> Result<Draft, CoreError> {
    Ok(Draft {
        id: row.try_get("id")?,
        context_kind: row.try_get("context_kind")?,
        context_id: row.try_get("context_id")?,
        generation: row.try_get("generation")?,
        version: row.try_get("version")?,
        consumed_version: row.try_get("consumed_version")?,
        state: row.try_get("state")?,
        payload: json!({"schemaVersion":row.try_get::<i16,_>("payload_schema_version")?,"body":row.try_get::<Value,_>("payload")?}),
        updated_at: row.try_get("updated_at")?,
    })
}

const COLUMNS: &str = "id,context_kind,context_id,generation,version,consumed_version,state,payload_schema_version,payload,updated_at";

pub async fn save_draft(
    pool: &PgPool,
    actor_id: Uuid,
    kind: &str,
    context_id: Uuid,
    expected_version: Option<i64>,
    body: Value,
) -> Result<(Draft, bool), CoreError> {
    let mut text = String::new();
    plain_text(&body, &mut text);
    if text.trim().is_empty() {
        return Err(CoreError::validation(
            "an empty draft is not stored; delete it instead",
        ));
    }
    if body.to_string().len() > 200_000 {
        return Err(CoreError::validation("the draft is too large"));
    }
    let mut tx = pool.begin().await?;
    let (workspace_id, _, _, kind) = context(&mut tx, actor_id, kind, context_id).await?;
    let existing = sqlx::query(&format!(
        "SELECT {COLUMNS} FROM penfii.drafts WHERE user_id=$1 AND workspace_id=$2 AND context_kind=$3 AND context_id=$4 FOR UPDATE"
    ))
    .bind(actor_id)
    .bind(workspace_id)
    .bind(kind)
    .bind(context_id)
    .fetch_optional(&mut *tx)
    .await?;
    let (draft, created) = match existing {
        Some(row) => {
            let current: i64 = row.try_get("version")?;
            if expected_version.is_some_and(|v| v != current) {
                return Err(CoreError::conflict_extra(
                    "draft_conflict",
                    "this draft changed elsewhere",
                    serde_json::from_value(json!({"currentVersion":current})).unwrap_or_default(),
                ));
            }
            let id: Uuid = row.try_get("id")?;
            let updated = sqlx::query(&format!(
                "UPDATE penfii.drafts SET payload=$2,payload_schema_version=1,version=version+1,state='ACTIVE',updated_at=now() WHERE id=$1 RETURNING {COLUMNS}"
            ))
            .bind(id)
            .bind(&body)
            .fetch_one(&mut *tx)
            .await?;
            (draft_from(&updated)?, false)
        }
        None => {
            let inserted = sqlx::query(&format!(
                "INSERT INTO penfii.drafts(id,user_id,workspace_id,context_kind,context_id,generation,payload_schema_version,payload)
                 VALUES($1,$2,$3,$4,$5,$6,1,$7) RETURNING {COLUMNS}"
            ))
            .bind(Uuid::new_v4())
            .bind(actor_id)
            .bind(workspace_id)
            .bind(kind)
            .bind(context_id)
            .bind(Uuid::new_v4())
            .bind(&body)
            .fetch_one(&mut *tx)
            .await?;
            (draft_from(&inserted)?, true)
        }
    };
    tx.commit().await?;
    Ok((draft, created))
}

pub async fn load_draft(
    pool: &PgPool,
    actor_id: Uuid,
    kind: &str,
    context_id: Uuid,
) -> Result<Option<Draft>, CoreError> {
    let mut tx = pool.begin().await?;
    let (workspace_id, _, _, kind) = context(&mut tx, actor_id, kind, context_id).await?;
    let row = sqlx::query(&format!(
        "SELECT {COLUMNS} FROM penfii.drafts WHERE user_id=$1 AND workspace_id=$2 AND context_kind=$3 AND context_id=$4 AND state='ACTIVE'"
    ))
    .bind(actor_id)
    .bind(workspace_id)
    .bind(kind)
    .bind(context_id)
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    row.as_ref().map(draft_from).transpose()
}

pub async fn delete_draft(pool: &PgPool, actor_id: Uuid, draft_id: Uuid) -> Result<(), CoreError> {
    let deleted = sqlx::query("DELETE FROM penfii.drafts WHERE id=$1 AND user_id=$2")
        .bind(draft_id)
        .bind(actor_id)
        .execute(pool)
        .await?;
    if deleted.rows_affected() == 0 {
        return Err(CoreError::NotFound);
    }
    Ok(())
}

/// The caller's Drafts in a Workspace, newest first. A Draft in a conversation the caller can no
/// longer write to is left out, and so is everything about it.
pub async fn list_drafts(
    pool: &PgPool,
    actor_id: Uuid,
    workspace_id: Uuid,
) -> Result<Vec<DraftItem>, CoreError> {
    let mut tx = pool.begin().await?;
    access::require_member(&mut tx, workspace_id, actor_id).await?;
    let rows = sqlx::query(
        "SELECT id,context_kind,context_id,payload,updated_at FROM penfii.drafts
         WHERE user_id=$1 AND workspace_id=$2 AND state='ACTIVE' ORDER BY updated_at DESC, id LIMIT 200",
    )
    .bind(actor_id)
    .bind(workspace_id)
    .fetch_all(&mut *tx)
    .await?;
    let mut items = Vec::new();
    for row in rows {
        let kind: String = row.try_get("context_kind")?;
        let context_id: Uuid = row.try_get("context_id")?;
        let Ok((_, conversation_id, thread, _)) =
            context(&mut tx, actor_id, &kind, context_id).await
        else {
            continue;
        };
        let channel = sqlx::query("SELECT name,project_id FROM penfii.channels WHERE id=$1")
            .bind(conversation_id)
            .fetch_optional(&mut *tx)
            .await?;
        let body: Value = row.try_get("payload")?;
        let mut text = String::new();
        plain_text(&body, &mut text);
        let preview: String = text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(PREVIEW_CHARS)
            .collect();
        items.push(DraftItem {
            id: row.try_get("id")?,
            context_kind: kind,
            context_id,
            conversation_id,
            thread_root_id: thread,
            title: channel.as_ref().map(|c| c.get::<String, _>("name")),
            project_id: channel
                .as_ref()
                .and_then(|c| c.get::<Option<Uuid>, _>("project_id")),
            preview,
            updated_at: row.try_get("updated_at")?,
        });
    }
    tx.commit().await?;
    Ok(items)
}
