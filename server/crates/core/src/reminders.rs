//! Reminders: a time on a Later item. One-shot, absolute, with the time zone the person chose.
//!
//! A scheduler polls for due Reminders. Each is handled in its own transaction under a row lock
//! (`SKIP LOCKED`), so two schedulers never fire the same one. Firing writes an event; the Inbox
//! consumer turns it into the person's item, keyed by the Reminder so a repeat cannot double it.

use chrono::{DateTime, Duration, Utc};
use penfii_contracts::ReminderInfo;
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool, Row};
use uuid::Uuid;

use crate::error::CoreError;
use crate::events::{self, EventRow, NewEvent};
use crate::notify::{Candidate, Subject};
use crate::visibility;

const LATE_AFTER_SECS: i64 = 90;

fn info(row: &sqlx::postgres::PgRow) -> Result<ReminderInfo, CoreError> {
    Ok(ReminderInfo {
        id: row.try_get("id")?,
        due_at: row.try_get("due_at")?,
        time_zone: row.try_get("time_zone")?,
        state: row.try_get("state")?,
        version: row.try_get("version")?,
        fired_at: row.try_get("fired_at")?,
    })
}

/// The Reminder worth showing on a Later item: the scheduled one, else the most recent finished.
pub async fn latest(
    conn: &mut PgConnection,
    later_id: Uuid,
) -> Result<Option<ReminderInfo>, CoreError> {
    let row = sqlx::query(
        "SELECT id,due_at,time_zone,state,version,fired_at FROM penfii.reminders WHERE later_id=$1
         ORDER BY (state='SCHEDULED') DESC, created_at DESC LIMIT 1",
    )
    .bind(later_id)
    .fetch_optional(&mut *conn)
    .await?;
    row.as_ref().map(info).transpose()
}

pub async fn cancel_for_later(
    conn: &mut PgConnection,
    later_id: Uuid,
    reason: &str,
) -> Result<(), CoreError> {
    sqlx::query("UPDATE penfii.reminders SET state='CANCELED', cancel_reason=$2 WHERE later_id=$1 AND state='SCHEDULED'")
        .bind(later_id)
        .bind(reason)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

async fn target_visible(
    conn: &mut PgConnection,
    user: Uuid,
    kind: &str,
    id: Uuid,
) -> Result<bool, CoreError> {
    Ok(match kind {
        "TICKET" => visibility::tickets(conn, user, &[id]).await?.contains(&id),
        "MESSAGE" => visibility::messages(conn, user, &[id]).await?.contains(&id),
        "FILE" => visibility::files(conn, user, &[id]).await?.contains(&id),
        _ => false,
    })
}

/// Sets (or replaces) the Reminder of an active Later item.
pub async fn set(
    pool: &PgPool,
    actor: Uuid,
    later_id: Uuid,
    due_at: DateTime<Utc>,
    time_zone: &str,
) -> Result<ReminderInfo, CoreError> {
    let now = Utc::now();
    if due_at < now - Duration::minutes(1) {
        return Err(CoreError::validation("choose a time that has not passed"));
    }
    if due_at > now + Duration::days(366) {
        return Err(CoreError::validation(
            "a Reminder can be at most a year ahead",
        ));
    }
    let mut tx = pool.begin().await?;
    let known: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_timezone_names WHERE name=$1)")
            .bind(time_zone)
            .fetch_one(&mut *tx)
            .await?;
    if !known {
        return Err(CoreError::validation("unknown time zone"));
    }
    let later = sqlx::query(
        "SELECT workspace_id,kind,target_id,state FROM penfii.later_items WHERE id=$1 AND user_id=$2 FOR UPDATE",
    )
    .bind(later_id)
    .bind(actor)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(CoreError::NotFound)?;
    let (workspace, kind, target): (Uuid, String, Uuid) = (
        later.try_get("workspace_id")?,
        later.try_get("kind")?,
        later.try_get("target_id")?,
    );
    if later.try_get::<String, _>("state")? != "ACTIVE" {
        return Err(CoreError::conflict(
            "later_closed",
            "this item is finished; bring it back first",
        ));
    }
    if !target_visible(&mut tx, actor, &kind, target).await? {
        return Err(CoreError::NotFound);
    }
    let previous: Option<i64> = sqlx::query_scalar(
        "SELECT version FROM penfii.reminders WHERE later_id=$1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(later_id)
    .fetch_optional(&mut *tx)
    .await?;
    cancel_for_later(&mut tx, later_id, "REPLACED").await?;
    let row = sqlx::query(
        "INSERT INTO penfii.reminders(id,user_id,workspace_id,target_kind,target_id,later_id,due_at,time_zone,version)
         VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9) RETURNING id,due_at,time_zone,state,version,fired_at",
    )
    .bind(Uuid::new_v4())
    .bind(actor)
    .bind(workspace)
    .bind(&kind)
    .bind(target)
    .bind(later_id)
    .bind(due_at)
    .bind(time_zone)
    .bind(previous.unwrap_or(0) + 1)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    info(&row)
}

pub async fn cancel(pool: &PgPool, actor: Uuid, later_id: Uuid) -> Result<(), CoreError> {
    let mut tx = pool.begin().await?;
    let owned: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM penfii.later_items WHERE id=$1 AND user_id=$2 FOR UPDATE",
    )
    .bind(later_id)
    .bind(actor)
    .fetch_optional(&mut *tx)
    .await?;
    owned.ok_or(CoreError::NotFound)?;
    cancel_for_later(&mut tx, later_id, "CANCELED").await?;
    tx.commit().await?;
    Ok(())
}

/// Fires what is due. Returns how many Reminders were handled (fired or canceled).
pub async fn fire_due(
    pool: &PgPool,
    limit: i64,
    workspace: Option<Uuid>,
) -> Result<usize, CoreError> {
    let mut tx = pool.begin().await?;
    let rows = sqlx::query(
        "SELECT r.id, r.user_id, r.workspace_id, r.target_kind, r.target_id, r.later_id, r.due_at, l.state AS later_state
         FROM penfii.reminders r JOIN penfii.later_items l ON l.id=r.later_id
         WHERE r.state='SCHEDULED' AND r.due_at <= now() AND ($2::uuid IS NULL OR r.workspace_id=$2)
         ORDER BY r.due_at, r.id
         FOR UPDATE OF r SKIP LOCKED
         LIMIT $1",
    )
    .bind(limit)
    .bind(workspace)
    .fetch_all(&mut *tx)
    .await?;
    for row in &rows {
        let id: Uuid = row.try_get("id")?;
        let user: Uuid = row.try_get("user_id")?;
        let kind: String = row.try_get("target_kind")?;
        let target: Uuid = row.try_get("target_id")?;
        let later_active = row.try_get::<String, _>("later_state")? == "ACTIVE";
        let reason = if !later_active {
            Some("LATER_CLOSED")
        } else if !target_visible(&mut tx, user, &kind, target).await? {
            Some("ACCESS_LOST")
        } else {
            None
        };
        if let Some(reason) = reason {
            sqlx::query(
                "UPDATE penfii.reminders SET state='CANCELED', cancel_reason=$2 WHERE id=$1",
            )
            .bind(id)
            .bind(reason)
            .execute(&mut *tx)
            .await?;
            continue;
        }
        let due: DateTime<Utc> = row.try_get("due_at")?;
        let late = (Utc::now() - due).num_seconds().max(0);
        sqlx::query("UPDATE penfii.reminders SET state='FIRED', fired_at=now() WHERE id=$1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        events::emit(
            &mut tx,
            NewEvent {
                workspace_id: row.try_get("workspace_id")?,
                target_kind: "REMINDER",
                target_id: id,
                target_version: None,
                kind: "reminder.due",
                actor_user_id: None,
                command_id: Uuid::new_v4(),
                facts: json!({
                    "userId": user, "reminderId": id, "laterId": row.try_get::<Uuid, _>("later_id")?,
                    "targetKind": kind, "targetId": target, "lateSeconds": late,
                }),
                security_context: json!({}),
            },
        )
        .await?;
    }
    tx.commit().await?;
    Ok(rows.len())
}

pub fn spawn(pool: PgPool) -> tokio::task::JoinHandle<()> {
    crate::lifecycle::spawn_worker("reminder scheduler", async move {
        while !crate::lifecycle::stopping() {
            match crate::lifecycle::guarded("reminder scheduler", fire_due(&pool, 50, None)).await {
                Ok(0) => crate::lifecycle::pause(std::time::Duration::from_secs(15)).await,
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(?error, "reminder scheduler failed");
                    crate::lifecycle::pause(std::time::Duration::from_secs(15)).await;
                }
            }
        }
    })
}

/// The Inbox side: the Reminder's owner, if they can still see its target.
pub(crate) async fn due_candidates(
    conn: &mut PgConnection,
    event: &EventRow,
) -> Result<(Option<Subject>, Vec<Candidate>), CoreError> {
    let (Some(user), Some(reminder)) = (
        event
            .facts
            .get("userId")
            .and_then(Value::as_str)
            .and_then(|s| Uuid::parse_str(s).ok()),
        event
            .facts
            .get("reminderId")
            .and_then(Value::as_str)
            .and_then(|s| Uuid::parse_str(s).ok()),
    ) else {
        return Ok((None, Vec::new()));
    };
    let kind = event.facts["targetKind"].as_str().unwrap_or_default();
    let Some(target) = event
        .facts
        .get("targetId")
        .and_then(Value::as_str)
        .and_then(|s| Uuid::parse_str(s).ok())
    else {
        return Ok((None, Vec::new()));
    };
    let late = event.facts["lateSeconds"].as_i64().unwrap_or(0);
    let mut params = json!({"reminderId": reminder, "targetKind": kind});
    if late > LATE_AFTER_SECS {
        params["lateSeconds"] = json!(late);
    }
    let subject = match kind {
        "TICKET" => {
            params["projectId"] = json!(
                sqlx::query_scalar::<_, Uuid>("SELECT project_id FROM penfii.tickets WHERE id=$1")
                    .bind(target)
                    .fetch_optional(&mut *conn)
                    .await?
            );
            Subject::Ticket(target)
        }
        "MESSAGE" => {
            let row = sqlx::query(
                "SELECT conversation_id, thread_root_id FROM penfii.messages WHERE id=$1",
            )
            .bind(target)
            .fetch_optional(&mut *conn)
            .await?;
            if let Some(row) = row {
                params["conversationId"] = json!(row.try_get::<Uuid, _>("conversation_id")?);
                params["messageId"] = json!(target);
                params["threadRootId"] = json!(row.try_get::<Option<Uuid>, _>("thread_root_id")?);
            }
            Subject::Message(target)
        }
        "FILE" => {
            let places = sqlx::query(
                "SELECT m.id, m.conversation_id, m.thread_root_id FROM penfii.message_attachments a
                 JOIN penfii.messages m ON m.id=a.message_id AND m.deleted_at IS NULL WHERE a.file_id=$1 ORDER BY m.created_at",
            )
            .bind(target)
            .fetch_all(&mut *conn)
            .await?;
            for place in places {
                let conversation: Uuid = place.try_get("conversation_id")?;
                if crate::access::load_conversation(conn, user, conversation)
                    .await?
                    .is_some_and(|a| a.can_read())
                {
                    params["conversationId"] = json!(conversation);
                    params["messageId"] = json!(place.try_get::<Uuid, _>("id")?);
                    params["threadRootId"] =
                        json!(place.try_get::<Option<Uuid>, _>("thread_root_id")?);
                    break;
                }
            }
            Subject::File(target)
        }
        _ => return Ok((None, Vec::new())),
    };
    Ok((
        Some(subject),
        vec![Candidate {
            subject: None,
            recipient: user,
            kind: "reminder.due",
            logical_key: format!("reminder:{reminder}"),
            params,
        }],
    ))
}
