//! Ticket trash and purge.
//!
//! Trash is the ordinary way a Ticket leaves the lists: a Project admin moves it to the trash, it
//! disappears from boards, lists, search and reports, and the same admin can restore it. Purge is the
//! administrative, irreversible step for a Ticket already in the trash and belongs to the Project's admin
//! (the Workspace role opens no Project): it erases the content (title, description, revisions, comments, activity, event facts and
//! everyone's personal references) and keeps only a marker row (the key and who purged it, when) so
//! links and counters do not dangle. Attached files are erased with it. A Ticket that still has children or dependencies cannot be purged;
//! the admin resolves those first.

use chrono::{DateTime, Utc};
use penfii_contracts::{ArchivedTicket, TrashedTicket};
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool, Row};
use uuid::Uuid;

use crate::admin;
use crate::error::CoreError;
use crate::events;
use crate::files::ObjectStore;
use crate::platform::ChangeNotice;
use crate::project_role;
use crate::ticket_files;

pub(crate) struct TicketState {
    workspace_id: Uuid,
    project_id: Uuid,
    key: String,
    trashed_at: Option<DateTime<Utc>>,
    purged_at: Option<DateTime<Utc>>,
    archived_at: Option<DateTime<Utc>>,
}

/// Writes the change record of a lifecycle step into the Project's change feed (inside the caller's
/// transaction) and returns the notice that makes open boards and Ticket panels look again. Like every
/// other Ticket command, the caller publishes the notice after the commit.
async fn lifecycle_notice(
    conn: &mut PgConnection,
    ticket: &TicketState,
    ticket_id: Uuid,
    kind: &str,
) -> Result<ChangeNotice, CoreError> {
    crate::work::comment_change(conn, ticket.project_id, ticket_id, kind).await
}

/// The Ticket row with a lock; a Ticket the caller cannot see at all is "not found".
pub(crate) async fn locked_ticket(
    conn: &mut PgConnection,
    ticket_id: Uuid,
) -> Result<TicketState, CoreError> {
    let row = sqlx::query(
        "SELECT workspace_id, project_id, public_key, trashed_at, purged_at, archived_at
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
        archived_at: row.try_get("archived_at")?,
    })
}

/// Moves a live Ticket to the trash. Project admin only. Children and other Tickets are not touched.
/// The notice tells open boards of the Project to look again; publish it after this returns.
pub async fn trash_ticket(
    pool: &PgPool,
    actor: Uuid,
    ticket_id: Uuid,
) -> Result<ChangeNotice, CoreError> {
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
    let notice = lifecycle_notice(&mut tx, &ticket, ticket_id, "ticket.trashed").await?;
    tx.commit().await?;
    Ok(notice)
}

/// Brings a trashed Ticket back. Project admin only.
pub async fn restore_from_trash(
    pool: &PgPool,
    actor: Uuid,
    ticket_id: Uuid,
) -> Result<ChangeNotice, CoreError> {
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
    let notice =
        lifecycle_notice(&mut tx, &ticket, ticket_id, "ticket.restored_from_trash").await?;
    tx.commit().await?;
    Ok(notice)
}

/// What an archive did: a Ticket that was already archived is left as it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveOutcome {
    Archived,
    AlreadyArchived,
}

/// Archives a live Ticket inside the caller's transaction. The caller has checked who may do it.
/// `scheduled` marks an archive a Sprint schedule ran rather than a person's own click.
pub(crate) async fn archive_in_tx(
    tx: &mut PgConnection,
    actor: Uuid,
    ticket_id: Uuid,
    scheduled: bool,
) -> Result<ArchiveOutcome, CoreError> {
    let ticket = locked_ticket(tx, ticket_id).await?;
    archive_locked(tx, actor, ticket_id, &ticket, scheduled).await
}

/// The archive itself, for a Ticket already locked by the caller. A Ticket with an open (live, not
/// DONE/CANCELED) child is refused: the parent stays while work remains.
pub(crate) async fn archive_locked(
    tx: &mut PgConnection,
    actor: Uuid,
    ticket_id: Uuid,
    ticket: &TicketState,
    scheduled: bool,
) -> Result<ArchiveOutcome, CoreError> {
    if ticket.purged_at.is_some() {
        return Err(CoreError::NotFound);
    }
    if ticket.trashed_at.is_some() {
        return Err(CoreError::conflict(
            "ticket_trashed",
            "restore the ticket from the trash before archiving it",
        ));
    }
    if ticket.archived_at.is_some() {
        return Ok(ArchiveOutcome::AlreadyArchived);
    }
    let open_children: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM penfii.tickets c
                       JOIN penfii.workflow_statuses s ON s.id = c.status_id
                       WHERE c.parent_id = $1 AND c.archived_at IS NULL AND c.trashed_at IS NULL
                         AND c.purged_at IS NULL AND s.category NOT IN ('DONE', 'CANCELED'))",
    )
    .bind(ticket_id)
    .fetch_one(&mut *tx)
    .await?;
    if open_children {
        return Err(CoreError::conflict(
            "open_children",
            "the ticket has unfinished child tickets",
        ));
    }
    sqlx::query("UPDATE penfii.tickets SET archived_at = now(), archived_by = $2 WHERE id = $1")
        .bind(ticket_id)
        .bind(actor)
        .execute(&mut *tx)
        .await?;
    admin::audit(
        &mut *tx,
        ticket.workspace_id,
        Some(actor),
        "ticket.archived",
        "TICKET",
        ticket_id,
        json!({"key": ticket.key, "projectId": ticket.project_id, "scheduled": scheduled}),
    )
    .await?;
    events::emit_simple(
        &mut *tx,
        ticket.workspace_id,
        "TICKET",
        ticket_id,
        "ticket.archived",
        Some(actor),
        json!({"scheduled": scheduled}),
    )
    .await?;
    Ok(ArchiveOutcome::Archived)
}

/// Archives a Ticket: it leaves the default lists and search, stays readable and becomes read-only.
/// Project admin only, like the trash. This is a Project-level lifecycle power, not a read: it does not depend on
/// the Ticket's channel and changes nothing about who can see the Ticket.
pub async fn archive_ticket(
    pool: &PgPool,
    actor: Uuid,
    ticket_id: Uuid,
) -> Result<(ArchiveOutcome, Option<ChangeNotice>), CoreError> {
    let mut tx = pool.begin().await?;
    let ticket = locked_ticket(&mut tx, ticket_id).await?;
    project_role::require_project_admin(&mut tx, ticket.project_id, actor).await?;
    let outcome = archive_locked(&mut tx, actor, ticket_id, &ticket, false).await?;
    // A Ticket that was already archived did not change: nothing to tell.
    let notice = if outcome == ArchiveOutcome::Archived {
        Some(lifecycle_notice(&mut tx, &ticket, ticket_id, "ticket.archived").await?)
    } else {
        None
    };
    tx.commit().await?;
    Ok((outcome, notice))
}

/// Brings an archived Ticket back to the live lists. Project admin only. A Ticket that is also in
/// the trash is restored from the trash first; the archive is not lifted behind its back.
pub async fn unarchive_ticket(
    pool: &PgPool,
    actor: Uuid,
    ticket_id: Uuid,
) -> Result<ChangeNotice, CoreError> {
    let mut tx = pool.begin().await?;
    let ticket = locked_ticket(&mut tx, ticket_id).await?;
    project_role::require_project_admin(&mut tx, ticket.project_id, actor).await?;
    if ticket.purged_at.is_some() {
        return Err(CoreError::NotFound);
    }
    if ticket.archived_at.is_none() {
        return Err(CoreError::conflict(
            "not_archived",
            "the ticket is not archived",
        ));
    }
    if ticket.trashed_at.is_some() {
        return Err(CoreError::conflict(
            "ticket_trashed",
            "restore the ticket from the trash before unarchiving it",
        ));
    }
    sqlx::query("UPDATE penfii.tickets SET archived_at = NULL, archived_by = NULL WHERE id = $1")
        .bind(ticket_id)
        .execute(&mut *tx)
        .await?;
    admin::audit(
        &mut tx,
        ticket.workspace_id,
        Some(actor),
        "ticket.unarchived",
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
        "ticket.unarchived",
        Some(actor),
        json!({}),
    )
    .await?;
    let notice = lifecycle_notice(&mut tx, &ticket, ticket_id, "ticket.unarchived").await?;
    tx.commit().await?;
    Ok(notice)
}

/// The Project's archived Tickets, newest first, for anyone who can read the Project; a Ticket in a
/// private channel the reader cannot see stays hidden.
pub async fn list_archived(
    pool: &PgPool,
    actor: Uuid,
    project_id: Uuid,
) -> Result<Vec<ArchivedTicket>, CoreError> {
    let mut conn = pool.acquire().await?;
    project_role::require_project_role(&mut conn, project_id, actor, false).await?;
    let rows = sqlx::query(
        "SELECT t.id, t.public_key, t.title, t.hierarchy_kind, t.channel_id, t.archived_at, t.archived_by,
                (t.trashed_at IS NOT NULL) AS trashed, u.display_name AS archived_by_name
         FROM penfii.tickets t
         LEFT JOIN penfii.users u ON u.id = t.archived_by
         WHERE t.project_id = $1 AND t.archived_at IS NOT NULL AND t.purged_at IS NULL
         ORDER BY t.archived_at DESC, t.id DESC
         LIMIT 500",
    )
    .bind(project_id)
    .fetch_all(&mut *conn)
    .await?;
    let mut out = Vec::new();
    for row in &rows {
        let channel_id: Option<Uuid> = row.try_get("channel_id")?;
        if !project_role::channel_allowed(&mut conn, channel_id, actor).await? {
            continue;
        }
        out.push(ArchivedTicket {
            id: row.try_get("id")?,
            key: row.try_get("public_key")?,
            title: row.try_get("title")?,
            hierarchy_kind: row.try_get("hierarchy_kind")?,
            archived_at: row.try_get("archived_at")?,
            archived_by: row.try_get("archived_by")?,
            archived_by_name: row.try_get("archived_by_name")?,
            trashed: row.try_get("trashed")?,
        });
    }
    Ok(out)
}

/// The Project's trash, newest first. Project admin only. Each row says whether the Ticket was also
/// archived (a restore brings it back archived), when the automatic purge takes it, and why that is on hold.
pub async fn list_trash(
    pool: &PgPool,
    actor: Uuid,
    project_id: Uuid,
) -> Result<Vec<TrashedTicket>, CoreError> {
    let mut conn = pool.acquire().await?;
    project_role::require_project_admin(&mut conn, project_id, actor).await?;
    let retention = trash_retention_days();
    let rows = sqlx::query(
        "SELECT t.id, t.public_key, t.title, t.hierarchy_kind, t.trashed_at, t.trashed_by,
                u.display_name AS trashed_by_name, (t.archived_at IS NOT NULL) AS archived,
                EXISTS(SELECT 1 FROM penfii.tickets c WHERE c.parent_id = t.id AND c.purged_at IS NULL) AS has_children,
                EXISTS(SELECT 1 FROM penfii.ticket_dependencies d
                       JOIN penfii.tickets o ON o.id = CASE WHEN d.predecessor_id = t.id THEN d.successor_id ELSE d.predecessor_id END
                       WHERE (d.predecessor_id = t.id OR d.successor_id = t.id) AND o.purged_at IS NULL) AS has_dependencies
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
            let trashed_at: DateTime<Utc> = row.try_get("trashed_at")?;
            let blocked = if row.try_get::<bool, _>("has_children")? {
                Some("children".to_owned())
            } else if row.try_get::<bool, _>("has_dependencies")? {
                Some("dependencies".to_owned())
            } else {
                None
            };
            Ok(TrashedTicket {
                id: row.try_get("id")?,
                key: row.try_get("public_key")?,
                title: row.try_get("title")?,
                hierarchy_kind: row.try_get("hierarchy_kind")?,
                trashed_at,
                trashed_by: row.try_get("trashed_by")?,
                trashed_by_name: row.try_get("trashed_by_name")?,
                archived: row.try_get("archived")?,
                purge_at: purge_at(trashed_at, retention),
                purge_blocked: blocked,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map_err(Into::into)
}

/// Why a trashed Ticket cannot be purged yet: it still has children or dependencies.
async fn purge_blocker(
    conn: &mut PgConnection,
    ticket_id: Uuid,
) -> Result<Option<CoreError>, CoreError> {
    let children: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.tickets WHERE parent_id = $1 AND purged_at IS NULL",
    )
    .bind(ticket_id)
    .fetch_one(&mut *conn)
    .await?;
    if children > 0 {
        return Ok(Some(CoreError::conflict(
            "ticket_has_children",
            "purge or move the child tickets first",
        )));
    }
    let dependencies: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.ticket_dependencies d
         WHERE (d.predecessor_id = $1 OR d.successor_id = $1)
           AND EXISTS (SELECT 1 FROM penfii.tickets o
                       WHERE o.id = CASE WHEN d.predecessor_id = $1 THEN d.successor_id ELSE d.predecessor_id END
                         AND o.purged_at IS NULL)",
    )
    .bind(ticket_id)
    .fetch_one(&mut *conn)
    .await?;
    if dependencies > 0 {
        return Ok(Some(CoreError::conflict(
            "ticket_has_dependencies",
            "remove the ticket's dependencies first",
        )));
    }
    Ok(None)
}

/// The purge itself, inside the caller's transaction, for a Ticket already checked to be in the trash
/// and free of children and dependencies. `actor` is `None` for the automatic sweep: the audit entry and
/// event then carry no person, and `automatic: true`; `purged_by` stays empty.
async fn purge_in_tx(
    tx: &mut PgConnection,
    store: Option<&ObjectStore>,
    actor: Option<Uuid>,
    ticket_id: Uuid,
    ticket: &TicketState,
) -> Result<(), CoreError> {
    scrub(tx, ticket_id, &ticket.key).await?;
    // Files are retired in this transaction. Their objects are removed after the commit, by the
    // storage consumer, so a slow store does not hold this transaction.
    let _keys = ticket_files::retire_all(tx, ticket_id, ticket.workspace_id, actor).await?;
    let _ = store;
    admin::audit(
        tx,
        ticket.workspace_id,
        actor,
        "ticket.purged",
        "TICKET",
        ticket_id,
        json!({"key": ticket.key, "projectId": ticket.project_id, "automatic": actor.is_none(),
               "archived": ticket.archived_at.is_some()}),
    )
    .await?;
    events::emit_simple(
        tx,
        ticket.workspace_id,
        "TICKET",
        ticket_id,
        "ticket.purged",
        actor,
        json!({}),
    )
    .await?;
    if let Some(actor) = actor {
        sqlx::query("UPDATE penfii.tickets SET purged_by = $2 WHERE id = $1")
            .bind(ticket_id)
            .bind(actor)
            .execute(&mut *tx)
            .await?;
    }
    Ok(())
}

/// Erases a trashed Ticket's content for good. Project admin only, like the trash it comes from.
pub async fn purge_ticket(
    pool: &PgPool,
    store: Option<&ObjectStore>,
    actor: Uuid,
    ticket_id: Uuid,
) -> Result<ChangeNotice, CoreError> {
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
    if let Some(blocker) = purge_blocker(&mut tx, ticket_id).await? {
        return Err(blocker);
    }
    purge_in_tx(&mut tx, store, Some(actor), ticket_id, &ticket).await?;
    let notice = lifecycle_notice(&mut tx, &ticket, ticket_id, "ticket.purged").await?;
    tx.commit().await?;
    Ok(notice)
}

/// How long a Ticket stays in the trash before the automatic purge takes it: `PENFII_TRASH_RETENTION_DAYS`.
/// Unset, `0`, empty or not a whole number mean **off**: a Ticket then stays until someone restores or purges
/// it, so an upgrade never deletes existing trash by itself. Only a positive value turns the sweep on.
pub fn trash_retention_days() -> i64 {
    retention_days_from(std::env::var("PENFII_TRASH_RETENTION_DAYS").ok().as_deref())
}

fn retention_days_from(raw: Option<&str>) -> i64 {
    raw.and_then(|value| value.trim().parse::<i64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(0)
}

/// When the automatic purge takes a Ticket trashed at `trashed_at`; none while the sweep is off.
fn purge_at(trashed_at: DateTime<Utc>, retention_days: i64) -> Option<DateTime<Utc>> {
    (retention_days > 0).then(|| trashed_at + chrono::Duration::days(retention_days))
}

/// What one automatic purge pass did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TrashSweep {
    pub purged: usize,
    /// Expired but kept: still has children or dependencies, or the purge failed and is retried next pass.
    pub held: usize,
}

/// Purges the Tickets that have been in the trash longer than `retention_days`, at most `limit` per pass.
/// Children go before their parents (Subtasks, then Tasks, then Epics) so a parent whose children expired
/// together is not held for a pass. Each Ticket is its own transaction and is judged again under its row
/// lock, so a Ticket restored meanwhile, or purged by another instance or an admin, is left alone. A
/// Ticket that still has children or dependencies is held and tried again on the next pass. Object
/// removal happens after the commit and does not roll the purge back.
pub async fn purge_expired(
    pool: &PgPool,
    store: Option<&ObjectStore>,
    retention_days: i64,
    limit: i64,
) -> Result<TrashSweep, CoreError> {
    let mut sweep = TrashSweep::default();
    if retention_days <= 0 {
        return Ok(sweep);
    }
    let due: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM penfii.tickets
         WHERE trashed_at IS NOT NULL AND purged_at IS NULL
           AND trashed_at < now() - make_interval(days => $1::int)
         ORDER BY CASE hierarchy_kind WHEN 'SUBTASK' THEN 0 WHEN 'TASK' THEN 1 ELSE 2 END, trashed_at, id
         LIMIT $2",
    )
    .bind(retention_days as i32)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    for ticket_id in due {
        let mut tx = pool.begin().await?;
        let ticket = match locked_ticket(&mut tx, ticket_id).await {
            Ok(ticket) => ticket,
            Err(CoreError::NotFound) => continue,
            Err(error) => return Err(error),
        };
        let expired: bool = sqlx::query_scalar(
            "SELECT trashed_at IS NOT NULL AND purged_at IS NULL
                    AND trashed_at < now() - make_interval(days => $2::int)
             FROM penfii.tickets WHERE id = $1",
        )
        .bind(ticket_id)
        .bind(retention_days as i32)
        .fetch_one(&mut *tx)
        .await?;
        if !expired {
            continue;
        }
        if purge_blocker(&mut tx, ticket_id).await?.is_some() {
            sweep.held += 1;
            continue;
        }
        match purge_in_tx(&mut tx, store, None, ticket_id, &ticket).await {
            Ok(()) => {
                tx.commit().await?;
                sweep.purged += 1;
            }
            Err(error) => {
                tracing::warn!(?error, %ticket_id, "automatic purge failed; it is retried on the next pass");
                sweep.held += 1;
            }
        }
    }
    Ok(sweep)
}

/// Runs [`purge_expired`] shortly after start (so a server that was down catches up) and then every
/// hour. Every instance runs it; the row locks keep a Ticket from being purged twice.
pub fn spawn_trash_sweeper(
    pool: PgPool,
    store: Option<ObjectStore>,
) -> tokio::task::JoinHandle<()> {
    crate::lifecycle::spawn_worker("trash sweeper", async move {
        crate::lifecycle::pause(std::time::Duration::from_secs(30)).await;
        while !crate::lifecycle::stopping() {
            let days = trash_retention_days();
            match crate::lifecycle::guarded(
                "trash sweeper",
                purge_expired(&pool, store.as_ref(), days, 200),
            )
            .await
            {
                Ok(sweep) if sweep.purged > 0 || sweep.held > 0 => {
                    tracing::info!(
                        purged = sweep.purged,
                        held = sweep.held,
                        "expired trash swept"
                    );
                }
                Ok(_) => {}
                Err(error) => tracing::warn!(?error, "trash sweep failed"),
            }
            crate::lifecycle::pause(std::time::Duration::from_secs(3600)).await;
        }
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_automatic_purge_is_off_unless_a_positive_number_of_days_is_set() {
        assert_eq!(retention_days_from(None), 0);
        for off in ["", " ", "0", "-5", "abc", "1.5", "off"] {
            assert_eq!(retention_days_from(Some(off)), 0, "{off:?}");
        }
        assert_eq!(retention_days_from(Some("30")), 30);
        assert_eq!(retention_days_from(Some(" 7 ")), 7);
    }

    #[test]
    fn a_ticket_has_a_purge_date_only_while_the_sweep_is_on() {
        let at = DateTime::parse_from_rfc3339("2026-11-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(purge_at(at, 0), None);
        assert_eq!(
            purge_at(at, 30),
            Some(
                DateTime::parse_from_rfc3339("2026-12-01T00:00:00Z")
                    .unwrap()
                    .with_timezone(&Utc)
            )
        );
    }
}
