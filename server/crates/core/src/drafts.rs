//! Drafts: the caller's unsent messages, kept on the server so they follow the person between
//! devices and can be listed in one place. A Draft belongs to one compose box (a channel, a thread
//! or a direct message). Saving over a newer version is refused so two tabs never silently
//! overwrite each other; sending (or emptying the box) ends the box's input session (`generation`), and a save that
//! belongs to an ended session, however late it arrives, does not bring the Draft back.

use penfii_contracts::{Draft, DraftItem};
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool, Row};
use std::collections::HashMap;
use uuid::Uuid;

use crate::access;
use crate::error::CoreError;

const KINDS: &[&str] = &["CHANNEL_COMPOSE", "THREAD_COMPOSE", "DM_COMPOSE"];
const PREVIEW_CHARS: usize = 140;

/// Text of a rich text document in reading order, for a preview and for telling blank from filled.
///
/// A mention counts as content (a box holding only a mention is not blank, the same as in the
/// client). It reads `@Name` when `names` knows the person and a bare `@` otherwise.
fn plain_text(value: &Value, names: &HashMap<Uuid, String>, out: &mut String) {
    if let Some(text) = value.get("text").and_then(Value::as_str) {
        out.push_str(text);
    }
    if value.get("type").and_then(Value::as_str) == Some("mention") {
        out.push('@');
        if let Some(name) = mentioned_user(value).and_then(|id| names.get(&id)) {
            out.push_str(name);
        }
    }
    if let Some(children) = value.get("content").and_then(Value::as_array) {
        for child in children {
            plain_text(child, names, out);
        }
        if matches!(
            value.get("type").and_then(Value::as_str),
            Some("paragraph" | "heading" | "codeBlock")
        ) {
            out.push(' ');
        }
    }
}

fn mentioned_user(node: &Value) -> Option<Uuid> {
    node.pointer("/attrs/userId")
        .and_then(Value::as_str)
        .and_then(|id| Uuid::parse_str(id).ok())
}

fn mentioned_users(value: &Value, out: &mut Vec<Uuid>) {
    if value.get("type").and_then(Value::as_str) == Some("mention") {
        out.extend(mentioned_user(value));
    }
    for child in value
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        mentioned_users(child, out);
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

/// Saves the caller's Draft of a compose box.
///
/// An input session has a `generation`: a UUID the client makes when the box goes from empty to typed, and that is
/// time-ordered (UUIDv7), so a later session sorts after an earlier one. The stored row keeps the newest generation it has
/// seen, and whether that session is still open (`ACTIVE`) or over (`SENT`: sent or discarded). That makes the order of
/// requests irrelevant:
///  - a save of a session that is over, or older than the stored one, is refused (`draft_closed`) and changes nothing: a save
///    that was still on its way when the message was sent cannot bring the text back, whether it is the first save of the
///    session (no version yet) or a later one, and whichever tab or reload it comes from;
///  - a save of the stored session is checked against `expected_version` as before (`draft_conflict`);
///  - a save of a newer session starts over from it (the person is typing here now).
///
/// A closed session is not deleted: the row stays, empty, as the memory of it (one row per box, as before).
pub async fn save_draft(
    pool: &PgPool,
    actor_id: Uuid,
    kind: &str,
    context_id: Uuid,
    generation: Uuid,
    expected_version: Option<i64>,
    body: Value,
) -> Result<(Draft, bool), CoreError> {
    let mut text = String::new();
    plain_text(&body, &HashMap::new(), &mut text);
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
    let select = format!(
        "SELECT {COLUMNS} FROM penfii.drafts WHERE user_id=$1 AND workspace_id=$2 AND context_kind=$3 AND context_id=$4 FOR UPDATE"
    );
    let mut existing = None;
    for _ in 0..2 {
        existing = sqlx::query(&select)
            .bind(actor_id)
            .bind(workspace_id)
            .bind(kind)
            .bind(context_id)
            .fetch_optional(&mut *tx)
            .await?;
        if existing.is_some() {
            break;
        }
        // Nothing to lock yet: two first saves of one box race on the unique key. The one that loses finds the other's row
        // on the second read and is judged against it.
        let inserted = sqlx::query(&format!(
            "INSERT INTO penfii.drafts(id,user_id,workspace_id,context_kind,context_id,generation,payload_schema_version,payload)
             VALUES($1,$2,$3,$4,$5,$6,1,$7)
             ON CONFLICT (user_id,workspace_id,context_kind,context_id) DO NOTHING RETURNING {COLUMNS}"
        ))
        .bind(Uuid::new_v4())
        .bind(actor_id)
        .bind(workspace_id)
        .bind(kind)
        .bind(context_id)
        .bind(generation)
        .bind(&body)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(row) = inserted {
            let draft = draft_from(&row)?;
            tx.commit().await?;
            return Ok((draft, true));
        }
    }
    let row = existing
        .ok_or_else(|| CoreError::Internal("draft row neither found nor created".into()))?;
    let stored: Uuid = row.try_get("generation")?;
    let state: String = row.try_get("state")?;
    let current: i64 = row.try_get("version")?;
    let id: Uuid = row.try_get("id")?;
    if generation < stored || (generation == stored && state != "ACTIVE") {
        return Err(CoreError::conflict_extra(
            "draft_closed",
            "this input session is over; a newer one starts a new draft",
            serde_json::from_value(json!({"currentGeneration":stored})).unwrap_or_default(),
        ));
    }
    if generation == stored && expected_version.is_some_and(|v| v != current) {
        return Err(CoreError::conflict_extra(
            "draft_conflict",
            "this draft changed elsewhere",
            serde_json::from_value(json!({"currentVersion":current,"currentGeneration":stored}))
                .unwrap_or_default(),
        ));
    }
    let created = state != "ACTIVE";
    let updated = sqlx::query(&format!(
        "UPDATE penfii.drafts SET generation=$2,payload=$3,payload_schema_version=1,version=version+1,state='ACTIVE',updated_at=now() WHERE id=$1 RETURNING {COLUMNS}"
    ))
    .bind(id)
    .bind(generation)
    .bind(&body)
    .fetch_one(&mut *tx)
    .await?;
    let draft = draft_from(&updated)?;
    tx.commit().await?;
    Ok((draft, created))
}

/// Ends an input session: the message was sent or the box was emptied. It works whether or not a save of the session has
/// arrived yet. Saves of this session, and of any older one, that come after it change nothing. Ending a session older than
/// the stored one does nothing (it is over already).
pub async fn close_draft(
    pool: &PgPool,
    actor_id: Uuid,
    kind: &str,
    context_id: Uuid,
    generation: Uuid,
) -> Result<(), CoreError> {
    let mut tx = pool.begin().await?;
    let (workspace_id, _, _, kind) = context(&mut tx, actor_id, kind, context_id).await?;
    sqlx::query(
        "INSERT INTO penfii.drafts(id,user_id,workspace_id,context_kind,context_id,generation,state,payload_schema_version,payload)
         VALUES($1,$2,$3,$4,$5,$6,'SENT',1,'{}'::jsonb)
         ON CONFLICT (user_id,workspace_id,context_kind,context_id) DO UPDATE
           SET generation=EXCLUDED.generation,state='SENT',payload='{}'::jsonb,version=penfii.drafts.version+1,updated_at=now()
           WHERE penfii.drafts.generation <= EXCLUDED.generation",
    )
    .bind(Uuid::new_v4())
    .bind(actor_id)
    .bind(workspace_id)
    .bind(kind)
    .bind(context_id)
    .bind(generation)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
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

/// Discards a Draft (from the Drafts list). Like a sent one it stays as the memory of its session, so a save that was
/// still on its way cannot bring it back.
pub async fn delete_draft(pool: &PgPool, actor_id: Uuid, draft_id: Uuid) -> Result<(), CoreError> {
    let closed = sqlx::query(
        "UPDATE penfii.drafts SET state='SENT',payload='{}'::jsonb,version=version+1,updated_at=now()
         WHERE id=$1 AND user_id=$2 AND state='ACTIVE'",
    )
    .bind(draft_id)
    .bind(actor_id)
    .execute(pool)
    .await?;
    if closed.rows_affected() == 0 {
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
        let mut ids = Vec::new();
        mentioned_users(&body, &mut ids);
        let names: HashMap<Uuid, String> = if ids.is_empty() {
            HashMap::new()
        } else {
            sqlx::query("SELECT id,display_name FROM penfii.users WHERE id = ANY($1)")
                .bind(&ids)
                .fetch_all(&mut *tx)
                .await?
                .into_iter()
                .map(|r| -> Result<_, sqlx::Error> {
                    Ok((r.try_get("id")?, r.try_get("display_name")?))
                })
                .collect::<Result<HashMap<_, _>, sqlx::Error>>()?
        };
        let mut text = String::new();
        plain_text(&body, &names, &mut text);
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
            title: channel
                .as_ref()
                .map(|c| c.try_get::<String, _>("name"))
                .transpose()?,
            project_id: channel
                .as_ref()
                .map(|c| c.try_get::<Option<Uuid>, _>("project_id"))
                .transpose()?
                .flatten(),
            preview,
            updated_at: row.try_get("updated_at")?,
        });
    }
    tx.commit().await?;
    Ok(items)
}
