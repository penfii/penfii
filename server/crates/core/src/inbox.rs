//! The Inbox read model and its commands.
//!
//! An item stores a kind, parameters and references. What is shown (titles, excerpts) is read now,
//! after asking the visibility service whether the person may still see the subject. An item whose
//! subject they cannot see is neither listed nor counted.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool, Row};
use uuid::Uuid;

use crate::access;
use crate::error::CoreError;
use crate::paging;
use crate::visibility;
use crate::work_history::description_text;

pub fn kinds_for_filter(filter: &str) -> Result<Option<Vec<&'static str>>, CoreError> {
    Ok(match filter {
        "" | "all" => None,
        "mentions" => Some(vec!["mention.user"]),
        "replies" => Some(vec!["reply.direct", "thread.reply"]),
        "watching" => Some(vec![
            "thread.reply",
            "ticket.commented",
            "ticket.status_changed",
            "ticket.field_changed",
            "sprint.started",
            "sprint.closed",
        ]),
        "reminders" => Some(vec!["reminder.due"]),
        "invitations" => Some(vec!["channel.invited"]),
        "assignments" => Some(vec!["ticket.assigned"]),
        _ => return Err(CoreError::validation("unknown Inbox filter")),
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxItem {
    pub id: Uuid,
    pub kind: String,
    pub state: String,
    pub snoozed_until: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub version: i64,
    pub actor: Option<Value>,
    pub subject: Value,
    pub params: Value,
    pub link: Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxPage {
    pub items: Vec<InboxItem>,
    pub next_cursor: Option<String>,
    pub unread_count: i64,
    /// True when more than `unread_count` unread items may exist (the count stops at 100).
    pub unread_capped: bool,
}

struct Row_ {
    id: Uuid,
    kind: String,
    state: String,
    snoozed_until: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    version: i64,
    decision: Value,
}

fn subject_of(decision: &Value) -> Option<(String, Uuid)> {
    let s = decision.get("subject")?;
    Some((
        s.get("kind")?.as_str()?.to_owned(),
        Uuid::parse_str(s.get("id")?.as_str()?).ok()?,
    ))
}

async fn visible_subjects(
    conn: &mut PgConnection,
    actor: Uuid,
    rows: &[Row_],
) -> Result<HashSet<(String, Uuid)>, CoreError> {
    let mut by_kind: HashMap<String, Vec<Uuid>> = HashMap::new();
    for row in rows {
        if let Some((kind, id)) = subject_of(&row.decision) {
            by_kind.entry(kind).or_default().push(id);
        }
    }
    let mut visible = HashSet::new();
    for (kind, ids) in by_kind {
        let found = match kind.as_str() {
            "TICKET" => visibility::tickets(conn, actor, &ids).await?,
            "MESSAGE" => visibility::messages(conn, actor, &ids).await?,
            "PROJECT" => visibility::projects(conn, actor, &ids).await?,
            "FILE" => visibility::files(conn, actor, &ids).await?,
            "DOC" => visibility::docs(conn, actor, &ids).await?,
            _ => HashSet::new(),
        };
        visible.extend(found.into_iter().map(|id| (kind.clone(), id)));
    }
    Ok(visible)
}

async fn load_rows(
    conn: &mut PgConnection,
    actor: Uuid,
    workspace_id: Uuid,
    kinds: Option<&[&str]>,
    unread_only: bool,
    after: Option<(DateTime<Utc>, Uuid)>,
    limit: i64,
) -> Result<Vec<Row_>, CoreError> {
    let kinds: Option<Vec<String>> = kinds.map(|k| k.iter().map(|s| (*s).to_owned()).collect());
    let rows = sqlx::query(
        "SELECT i.id, i.notification_kind, i.state, i.snoozed_until, i.created_at, i.version, r.decision
         FROM penfii.inbox_items i
         JOIN penfii.event_recipients r ON r.event_id=i.event_id AND r.recipient_id=i.recipient_id AND r.notification_kind=i.notification_kind
         WHERE i.recipient_id=$1 AND i.workspace_id=$2 AND i.state <> 'ARCHIVED'
           AND (i.snoozed_until IS NULL OR i.snoozed_until <= now())
           AND ($3::text[] IS NULL OR i.notification_kind = ANY($3))
           AND (NOT $4 OR i.state = 'UNREAD')
           AND ($5::timestamptz IS NULL OR (i.created_at, i.id) < ($5, $6))
         ORDER BY i.created_at DESC, i.id DESC
         LIMIT $7",
    )
    .bind(actor)
    .bind(workspace_id)
    .bind(kinds)
    .bind(unread_only)
    .bind(after.map(|a| a.0))
    .bind(after.map(|a| a.1).unwrap_or(Uuid::nil()))
    .bind(limit)
    .fetch_all(&mut *conn)
    .await?;
    rows.iter()
        .map(|row| {
            Ok(Row_ {
                id: row.try_get("id")?,
                kind: row.try_get("notification_kind")?,
                state: row.try_get("state")?,
                snoozed_until: row.try_get("snoozed_until")?,
                created_at: row.try_get("created_at")?,
                version: row.try_get("version")?,
                decision: row.try_get("decision")?,
            })
        })
        .collect()
}

pub async fn list(
    pool: &PgPool,
    actor: Uuid,
    workspace_id: Uuid,
    filter: &str,
    unread_only: bool,
    cursor: Option<&str>,
    limit: Option<i64>,
) -> Result<InboxPage, CoreError> {
    let limit = paging::require_limit(limit, 30, 100, "limit is 1 to 100")?;
    let kinds = kinds_for_filter(filter)?;
    let mut conn = pool.acquire().await?;
    access::require_member(&mut conn, workspace_id, actor).await?;
    let mut after = cursor.map(paging::decode_cursor_hex).transpose()?;
    let mut collected: Vec<Row_> = Vec::new();
    let mut next_cursor = None;
    let mut exhausted = false;
    for _ in 0..4 {
        let rows = load_rows(
            &mut conn,
            actor,
            workspace_id,
            kinds.as_deref(),
            unread_only,
            after,
            limit * 2,
        )
        .await?;
        if rows.is_empty() {
            exhausted = true;
            break;
        }
        let full = rows.len() as i64 >= limit * 2;
        let visible = visible_subjects(&mut conn, actor, &rows).await?;
        let last = rows.last().map(|r| (r.created_at, r.id));
        let mut stopped = false;
        for row in rows {
            let keep = subject_of(&row.decision).is_some_and(|s| visible.contains(&s));
            if !keep {
                continue;
            }
            let position = (row.created_at, row.id);
            collected.push(row);
            if collected.len() as i64 == limit {
                next_cursor = Some(paging::encode_cursor_hex(position.0, position.1));
                stopped = true;
                break;
            }
        }
        if stopped {
            break;
        }
        after = last;
        if !full {
            exhausted = true;
            break;
        }
    }
    if next_cursor.is_none()
        && !exhausted
        && let Some((at, id)) = after
    {
        next_cursor = Some(paging::encode_cursor_hex(at, id));
    }
    let items = decorate(&mut conn, actor, collected).await?;
    let (unread_count, unread_capped) = unread(&mut conn, actor, workspace_id).await?;
    Ok(InboxPage {
        items,
        next_cursor,
        unread_count,
        unread_capped,
    })
}

/// Unread items the person can still see, counted over the newest 100.
pub async fn unread(
    conn: &mut PgConnection,
    actor: Uuid,
    workspace_id: Uuid,
) -> Result<(i64, bool), CoreError> {
    let rows = load_rows(conn, actor, workspace_id, None, true, None, 101).await?;
    let capped = rows.len() > 100;
    let rows: Vec<Row_> = rows.into_iter().take(100).collect();
    let visible = visible_subjects(conn, actor, &rows).await?;
    let count = rows
        .iter()
        .filter(|r| subject_of(&r.decision).is_some_and(|s| visible.contains(&s)))
        .count() as i64;
    Ok((count, capped))
}

pub async fn unread_count(
    pool: &PgPool,
    actor: Uuid,
    workspace_id: Uuid,
) -> Result<(i64, bool), CoreError> {
    let mut conn = pool.acquire().await?;
    access::require_member(&mut conn, workspace_id, actor).await?;
    unread(&mut conn, actor, workspace_id).await
}

async fn decorate(
    conn: &mut PgConnection,
    actor: Uuid,
    rows: Vec<Row_>,
) -> Result<Vec<InboxItem>, CoreError> {
    let mut out = Vec::new();
    for row in rows {
        let Some((kind, id)) = subject_of(&row.decision) else {
            continue;
        };
        let title: Option<String> = match kind.as_str() {
            "TICKET" => sqlx::query("SELECT public_key, title FROM penfii.tickets WHERE id=$1")
                .bind(id)
                .fetch_optional(&mut *conn)
                .await?
                .map(|r| {
                    format!(
                        "{} {}",
                        r.get::<String, _>("public_key"),
                        r.get::<String, _>("title")
                    )
                }),
            "MESSAGE" => {
                sqlx::query("SELECT body FROM penfii.messages WHERE id=$1 AND deleted_at IS NULL")
                    .bind(id)
                    .fetch_optional(&mut *conn)
                    .await?
                    .map(|r| {
                        let text = description_text(&r.get::<Value, _>("body"));
                        let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
                        flat.chars().take(120).collect()
                    })
            }
            "PROJECT" => {
                sqlx::query_scalar("SELECT name FROM penfii.projects WHERE id=$1")
                    .bind(id)
                    .fetch_optional(&mut *conn)
                    .await?
            }
            "FILE" => {
                sqlx::query_scalar(
                    "SELECT filename FROM penfii.files WHERE id=$1 AND deleted_at IS NULL",
                )
                .bind(id)
                .fetch_optional(&mut *conn)
                .await?
            }
            "DOC" => {
                sqlx::query_scalar(
                    "SELECT title FROM penfii.doc_pages WHERE id=$1 AND deleted_at IS NULL",
                )
                .bind(id)
                .fetch_optional(&mut *conn)
                .await?
            }
            _ => None,
        };
        // Skipped where the subject is gone since the visibility check.
        let Some(title) = title else { continue };
        let actor_value = match row.decision.get("actorId").and_then(Value::as_str).and_then(|s| Uuid::parse_str(s).ok()) {
            Some(user) => sqlx::query("SELECT id, display_name FROM penfii.users WHERE id=$1")
                .bind(user)
                .fetch_optional(&mut *conn)
                .await?
                .map(|r| json!({"id": r.get::<Uuid, _>("id"), "name": r.get::<String, _>("display_name")})),
            None => None,
        };
        let link = match kind.as_str() {
            "TICKET" => {
                // Not every notification kind recorded the Project when it was made (a comment does not);
                // the Ticket knows it, and a link without it opens nothing.
                let recorded = row
                    .decision
                    .get("projectId")
                    .filter(|v| !v.is_null())
                    .cloned();
                let project = match recorded {
                    Some(value) => value,
                    None => sqlx::query_scalar::<_, Uuid>(
                        "SELECT project_id FROM penfii.tickets WHERE id=$1",
                    )
                    .bind(id)
                    .fetch_optional(&mut *conn)
                    .await?
                    .map_or(Value::Null, |p| json!(p)),
                };
                json!({"type": "ticket", "ticketId": id, "projectId": project})
            }
            "MESSAGE" => json!({
                "type": "message", "messageId": id,
                "conversationId": row.decision.get("conversationId"),
                "threadRootId": row.decision.get("threadRootId"),
            }),
            "FILE" => {
                json!({"type": "message", "conversationId": row.decision.get("conversationId"), "messageId": row.decision.get("messageId"), "threadRootId": row.decision.get("threadRootId")})
            }
            "DOC" => {
                json!({"type": "doc", "docId": id, "commentId": row.decision.get("commentId")})
            }
            _ => json!({"type": "project", "projectId": id}),
        };
        let mut params = row.decision.clone();
        if let Some(map) = params.as_object_mut() {
            map.remove("subject");
            map.remove("actorId");
        }
        let _ = actor;
        out.push(InboxItem {
            id: row.id,
            kind: row.kind,
            state: row.state,
            snoozed_until: row.snoozed_until,
            created_at: row.created_at,
            version: row.version,
            actor: actor_value,
            subject: json!({"kind": kind, "id": id, "title": title}),
            params,
            link,
        });
    }
    Ok(out)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Command {
    Read,
    Unread,
    Archive,
    Unarchive,
    Snooze(DateTime<Utc>),
}

pub async fn command(
    pool: &PgPool,
    actor: Uuid,
    item_id: Uuid,
    expected_version: i64,
    command: Command,
) -> Result<Value, CoreError> {
    let mut tx = pool.begin().await?;
    let row = sqlx::query(
        "SELECT state, snoozed_until, version FROM penfii.inbox_items WHERE id=$1 AND recipient_id=$2 FOR UPDATE",
    )
    .bind(item_id)
    .bind(actor)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(CoreError::NotFound)?;
    let (state, snoozed, version): (String, Option<DateTime<Utc>>, i64) = (
        row.try_get("state")?,
        row.try_get("snoozed_until")?,
        row.try_get("version")?,
    );
    if version != expected_version {
        let mut extra = serde_json::Map::new();
        extra.insert("currentVersion".into(), json!(version));
        return Err(CoreError::conflict_extra(
            "version_conflict",
            "this item changed",
            extra,
        ));
    }
    let (next_state, next_snooze) = match command {
        Command::Read => ("READ", None),
        Command::Unread => ("UNREAD", None),
        Command::Archive => ("ARCHIVED", None),
        Command::Unarchive => ("READ", None),
        Command::Snooze(until) => {
            if until <= Utc::now() {
                return Err(CoreError::validation("snooze until a time in the future"));
            }
            // Coming back, it is unread again.
            ("UNREAD", Some(until))
        }
    };
    let (version, state_out, snooze_out) = if next_state == state && next_snooze == snoozed {
        (version, state, snoozed)
    } else {
        sqlx::query("UPDATE penfii.inbox_items SET state=$2, snoozed_until=$3, version=version+1 WHERE id=$1")
            .bind(item_id)
            .bind(next_state)
            .bind(next_snooze)
            .execute(&mut *tx)
            .await?;
        (version + 1, next_state.to_owned(), next_snooze)
    };
    tx.commit().await?;
    Ok(json!({"id": item_id, "state": state_out, "snoozedUntil": snooze_out, "version": version}))
}

pub async fn read_all(
    pool: &PgPool,
    actor: Uuid,
    workspace_id: Uuid,
    filter: &str,
) -> Result<i64, CoreError> {
    let kinds: Option<Vec<String>> =
        kinds_for_filter(filter)?.map(|k| k.iter().map(|s| (*s).to_owned()).collect());
    let mut conn = pool.acquire().await?;
    access::require_member(&mut conn, workspace_id, actor).await?;
    let done = sqlx::query(
        "UPDATE penfii.inbox_items SET state='READ', version=version+1
         WHERE recipient_id=$1 AND workspace_id=$2 AND state='UNREAD'
           AND (snoozed_until IS NULL OR snoozed_until <= now())
           AND ($3::text[] IS NULL OR notification_kind = ANY($3))",
    )
    .bind(actor)
    .bind(workspace_id)
    .bind(kinds)
    .execute(&mut *conn)
    .await?;
    Ok(done.rows_affected() as i64)
}
