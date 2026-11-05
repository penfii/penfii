//! Ticket trash and purge.
//!
//! Trash is the ordinary way a Ticket leaves the lists: a Project admin moves it to the trash, it
//! disappears from boards, lists, search and reports, and the same admin can restore it. Purge is the
//! administrative, irreversible step for a Ticket already in the trash and belongs to Workspace owners and
//! admins: it erases the content (title, description, revisions, comments, activity, event facts and
//! everyone's personal references) and keeps only a marker row (the key and who purged it, when) so
//! links and counters do not dangle. Attached files are erased with it. A Ticket that still has children or dependencies cannot be purged;
//! the admin resolves those first.

use chrono::{DateTime, Utc};
use penfii_contracts::TrashedTicket;
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool, Row};
use uuid::Uuid;

use crate::admin;
use crate::error::CoreError;
use crate::events;
use crate::files::ObjectStore;
use crate::project_role;
use crate::ticket_files;

struct TicketState {
    workspace_id: Uuid,
    project_id: Uuid,
    key: String,
    trashed_at: Option<DateTime<Utc>>,
    purged_at: Option<DateTime<Utc>>,
}

/// The Ticket row with a lock; a Ticket the caller cannot see at all is "not found".
async fn locked_ticket(conn: &mut PgConnection, ticket_id: Uuid) -> Result<TicketState, CoreError> {
    let row = sqlx::query(
        "SELECT workspace_id, project_id, public_key, trashed_at, purged_at
         FROM penfii.tickets WHERE id = $1 FOR UPDATE",
    )
    .bind(ticket_id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(CoreError::NotFound)?;
    Ok(TicketState {
        workspace_id: row.try_get("workspace_id")?,
        project_id: row.try_get("project_id")?,
        key: row.try_get("public_key")?,
        trashed_at: row.try_get("trashed_at")?,
        purged_at: row.try_get("purged_at")?,
    })
}

/// Moves a live Ticket to the trash. Project admin only. Children and other Tickets are not touched.
pub async fn trash_ticket(pool: &PgPool, actor: Uuid, ticket_id: Uuid) -> Result<(), CoreError> {
    let mut tx = pool.begin().await?;
    let ticket = locked_ticket(&mut tx, ticket_id).await?;
    project_role::require_project_admin(&mut tx, ticket.project_id, actor).await?;
    if ticket.purged_at.is_some() {
        return Err(CoreError::NotFound);
    }
    if ticket.trashed_at.is_some() {
        return Err(CoreError::conflict(
            "already_trashed",
            "the ticket is already in the trash",
        ));
    }
    sqlx::query("UPDATE penfii.tickets SET trashed_at = now(), trashed_by = $2 WHERE id = $1")
        .bind(ticket_id)
        .bind(actor)
        .execute(&mut *tx)
        .await?;
    admin::audit(
        &mut tx,
        ticket.workspace_id,
        Some(actor),
        "ticket.trashed",
        "TICKET",
        ticket_id,
        json!({"key": ticket.key, "projectId": ticket.project_id}),
    )
    .await?;
    events::emit_simple(
        &mut tx,
        ticket.workspace_id,
        "TICKET",
        ticket_id,
        "ticket.trashed",
        Some(actor),
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Brings a trashed Ticket back. Project admin only.
pub async fn restore_from_trash(
    pool: &PgPool,
    actor: Uuid,
    ticket_id: Uuid,
) -> Result<(), CoreError> {
    let mut tx = pool.begin().await?;
    let ticket = locked_ticket(&mut tx, ticket_id).await?;
    project_role::require_project_admin(&mut tx, ticket.project_id, actor).await?;
    if ticket.purged_at.is_some() {
        return Err(CoreError::NotFound);
    }
    if ticket.trashed_at.is_none() {
        return Err(CoreError::conflict(
            "not_trashed",
            "the ticket is not in the trash",
        ));
    }
    sqlx::query("UPDATE penfii.tickets SET trashed_at = NULL, trashed_by = NULL WHERE id = $1")
        .bind(ticket_id)
        .execute(&mut *tx)
        .await?;
    admin::audit(
        &mut tx,
        ticket.workspace_id,
        Some(actor),
        "ticket.restored_from_trash",
        "TICKET",
        ticket_id,
        json!({"key": ticket.key, "projectId": ticket.project_id}),
    )
    .await?;
    events::emit_simple(
        &mut tx,
        ticket.workspace_id,
        "TICKET",
        ticket_id,
        "ticket.restored_from_trash",
        Some(actor),
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// The Project's trash, newest first. Project admin only.
pub async fn list_trash(
    pool: &PgPool,
    actor: Uuid,
    project_id: Uuid,
) -> Result<Vec<TrashedTicket>, CoreError> {
    let mut conn = pool.acquire().await?;
    project_role::require_project_admin(&mut conn, project_id, actor).await?;
    let rows = sqlx::query(
        "SELECT t.id, t.public_key, t.title, t.hierarchy_kind, t.trashed_at, t.trashed_by,
                u.display_name AS trashed_by_name
         FROM penfii.tickets t
         LEFT JOIN penfii.users u ON u.id = t.trashed_by
         WHERE t.project_id = $1 AND t.trashed_at IS NOT NULL AND t.purged_at IS NULL
         ORDER BY t.trashed_at DESC, t.id DESC
         LIMIT 500",
    )
    .bind(project_id)
    .fetch_all(&mut *conn)
    .await?;
    rows.iter()
        .map(|row| {
            Ok(TrashedTicket {
                id: row.try_get("id")?,
                key: row.try_get("public_key")?,
                title: row.try_get("title")?,
                hierarchy_kind: row.try_get("hierarchy_kind")?,
                trashed_at: row.try_get("trashed_at")?,
                trashed_by: row.try_get("trashed_by")?,
                trashed_by_name: row.try_get("trashed_by_name")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map_err(Into::into)
}

/// Erases a trashed Ticket's content for good. Project admin only, like the trash it comes from.
pub async fn purge_ticket(
    pool: &PgPool,
    store: Option<&ObjectStore>,
    actor: Uuid,
    ticket_id: Uuid,
) -> Result<(), CoreError> {
    let mut tx = pool.begin().await?;
    let ticket = locked_ticket(&mut tx, ticket_id).await?;
    // Erasing is Project management: the Workspace role opens no Project, so a Workspace admin who is not
    // the Project's admin is refused (or, outside it, told it does not exist).
    project_role::require_project_admin(&mut tx, ticket.project_id, actor).await?;
    if ticket.purged_at.is_some() {
        return Err(CoreError::NotFound);
    }
    if ticket.trashed_at.is_none() {
        return Err(CoreError::conflict(
            "not_trashed",
            "only a ticket in the trash can be purged",
        ));
    }
    let children: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.tickets WHERE parent_id = $1 AND purged_at IS NULL",
    )
    .bind(ticket_id)
    .fetch_one(&mut *tx)
    .await?;
    if children > 0 {
        return Err(CoreError::conflict(
            "ticket_has_children",
            "purge or move the child tickets first",
        ));
    }
    let dependencies: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.ticket_dependencies d
         WHERE (d.predecessor_id = $1 OR d.successor_id = $1)
           AND EXISTS (SELECT 1 FROM penfii.tickets o
                       WHERE o.id = CASE WHEN d.predecessor_id = $1 THEN d.successor_id ELSE d.predecessor_id END
                         AND o.purged_at IS NULL)",
    )
    .bind(ticket_id)
    .fetch_one(&mut *tx)
    .await?;
    if dependencies > 0 {
        return Err(CoreError::conflict(
            "ticket_has_dependencies",
            "remove the ticket's dependencies first",
        ));
    }
    scrub(&mut tx, ticket_id, &ticket.key).await?;
    // Files go with the content: the rows are dropped and the objects removed before the commit,
    // so a storage failure rolls the purge back to be repeated and no object outlives its row.
    let keys = ticket_files::retire_all(&mut tx, ticket_id, ticket.workspace_id, actor).await?;
    crate::files::delete_objects(store, &keys).await?;
    admin::audit(
        &mut tx,
        ticket.workspace_id,
        Some(actor),
        "ticket.purged",
        "TICKET",
        ticket_id,
        json!({"key": ticket.key, "projectId": ticket.project_id}),
    )
    .await?;
    events::emit_simple(
        &mut tx,
        ticket.workspace_id,
        "TICKET",
        ticket_id,
        "ticket.purged",
        Some(actor),
        json!({}),
    )
    .await?;
    sqlx::query("UPDATE penfii.tickets SET purged_by = $2 WHERE id = $1")
        .bind(ticket_id)
        .bind(actor)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

async fn scrub(conn: &mut PgConnection, ticket_id: Uuid, key: &str) -> Result<(), CoreError> {
    let marker: Value = json!({"purged": true, "key": key});
    sqlx::query(
        "UPDATE penfii.tickets
         SET title = '[purged]', description = '{}'::jsonb, assignee_id = NULL, priority = NULL,
             due_on = NULL, purged_at = now(), updated_at = now()
         WHERE id = $1",
    )
    .bind(ticket_id)
    .execute(&mut *conn)
    .await?;
    sqlx::query("UPDATE penfii.ticket_revisions SET snapshot = $2, restore_metadata = NULL WHERE ticket_id = $1")
        .bind(ticket_id)
        .bind(&marker)
        .execute(&mut *conn)
        .await?;
    sqlx::query("UPDATE penfii.ticket_comments SET body = '{}'::jsonb, deleted_at = COALESCE(deleted_at, now()) WHERE ticket_id = $1")
        .bind(ticket_id)
        .execute(&mut *conn)
        .await?;
    sqlx::query("UPDATE penfii.activity_entries SET data = '{}'::jsonb WHERE ticket_id = $1")
        .bind(ticket_id)
        .execute(&mut *conn)
        .await?;
    sqlx::query(
        "DELETE FROM penfii.inbox_items
         WHERE event_id IN (SELECT event_id FROM penfii.domain_events
                            WHERE target_kind = 'TICKET' AND target_id = $1)",
    )
    .bind(ticket_id)
    .execute(&mut *conn)
    .await?;
    sqlx::query("UPDATE penfii.domain_events SET facts = '{}'::jsonb WHERE target_kind = 'TICKET' AND target_id = $1")
        .bind(ticket_id)
        .execute(&mut *conn)
        .await?;
    for statement in [
        "DELETE FROM penfii.ticket_labels WHERE ticket_id = $1",
        "DELETE FROM penfii.ticket_milestones WHERE ticket_id = $1",
        "DELETE FROM penfii.ticket_schedules WHERE ticket_id = $1",
        "DELETE FROM penfii.project_pins WHERE ticket_id = $1",
        "DELETE FROM penfii.favorites WHERE kind = 'TICKET' AND target_id = $1",
        "DELETE FROM penfii.subscriptions WHERE target_kind = 'TICKET' AND target_id = $1",
        "DELETE FROM penfii.reminders WHERE target_kind = 'TICKET' AND target_id = $1",
        "DELETE FROM penfii.later_items WHERE kind = 'TICKET' AND target_id = $1",
        "DELETE FROM penfii.doc_page_targets WHERE target_kind = 'TICKET' AND target_id = $1",
    ] {
        sqlx::query(statement)
            .bind(ticket_id)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}
