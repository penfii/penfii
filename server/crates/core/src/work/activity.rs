//! A Ticket's activity feed and its revision history.

use penfii_contracts::{
    ActivityPage, RestorableField, RevisionDetail, RevisionPage, RevisionSummary, TicketActivity,
};
use serde_json::Value;
use sqlx::{PgConnection, PgPool, Row};
use std::collections::HashMap;
use uuid::Uuid;

use super::changes::{snapshot_value, status_names};
use super::restore::{live_ticket, restore_blocker};
use super::{PAGE_DEFAULT, PAGE_MAX, read_access};
use crate::error::CoreError;
use crate::paging;
use crate::project_role::{channel_allowed, require_project_role};
use crate::work_history::{self, RESTORABLE_FIELDS};

/// Normalized field views for the given revisions of one Ticket.
async fn revision_views(
    conn: &mut PgConnection,
    ticket_id: Uuid,
    revisions: &[i64],
    names: &HashMap<Uuid, String>,
) -> Result<HashMap<i64, Vec<work_history::FieldView>>, CoreError> {
    if revisions.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = sqlx::query(
        "SELECT revision_no,snapshot FROM penfii.ticket_revisions WHERE ticket_id=$1 AND revision_no=ANY($2)",
    )
    .bind(ticket_id)
    .bind(revisions)
    .fetch_all(&mut *conn)
    .await?;
    rows.into_iter()
        .map(|row| {
            let revision: i64 = row.try_get("revision_no")?;
            let snapshot: Value = row.try_get("snapshot")?;
            Ok((revision, work_history::view(&snapshot, names)))
        })
        .collect()
}

fn with_previous(revisions: impl Iterator<Item = i64>) -> Vec<i64> {
    let mut all: Vec<i64> = revisions
        .flat_map(|n| [n, n - 1])
        .filter(|n| *n > 0)
        .collect();
    all.sort_unstable();
    all.dedup();
    all
}

/// Activity: what happened to this Ticket, newest first. Field changes are derived from the
/// revision the event produced, so Activity and History cannot disagree.
/// Activity kinds that do not edit Ticket fields, so they carry no field diff.
pub(super) fn is_link_activity(kind: &str) -> bool {
    matches!(
        kind,
        "ticket.created"
            | "ticket.commented"
            | "ticket.dependency_added"
            | "ticket.dependency_removed"
            | "ticket.dependency_waived"
            | "ticket.dependency_restored"
            | "ticket.related_added"
            | "ticket.related_removed"
            | "ticket.sprint_changed"
            | "ticket.milestones_changed"
    )
}

/// Which OAuth client made a change, if it came through MCP: the Activity row or
/// revision is the person's own, and `mcp_call_log` says an MCP call produced it. Keys are
/// `<ticket id>:<revision>` for revisions and the comment id for comments. A change whose log
/// row has been purged simply has no marker.
async fn mcp_clients(
    conn: &mut PgConnection,
    revisions: &[(Uuid, i64)],
    comments: &[String],
) -> Result<HashMap<String, String>, CoreError> {
    if revisions.is_empty() && comments.is_empty() {
        return Ok(HashMap::new());
    }
    let tickets: Vec<String> = revisions.iter().map(|(id, _)| id.to_string()).collect();
    let numbers: Vec<i64> = revisions.iter().map(|(_, no)| *no).collect();
    let rows = sqlx::query(
        "SELECT DISTINCT ON (l.resource_type, l.resource_id, l.resource_revision)
                l.resource_type, l.resource_id, l.resource_revision, c.client_name
         FROM penfii.mcp_call_log l JOIN penfii.oauth_clients c ON c.id = l.client_pk
         WHERE l.outcome = 'ok'
           AND ((l.resource_type = 'ticket'
                 AND (l.resource_id, l.resource_revision) IN
                     (SELECT x.t, x.r FROM unnest($1::text[], $2::bigint[]) AS x(t, r)))
             OR (l.resource_type = 'ticket_comment' AND l.resource_id = ANY($3)))
         ORDER BY l.resource_type, l.resource_id, l.resource_revision, l.at",
    )
    .bind(&tickets)
    .bind(&numbers)
    .bind(comments)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let id: String = row.get("resource_id");
            let key = match row.get::<Option<i64>, _>("resource_revision") {
                Some(revision) => format!("{id}:{revision}"),
                None => id,
            };
            (key, row.get("client_name"))
        })
        .collect())
}

fn activity_via(
    clients: &HashMap<String, String>,
    kind: &str,
    ticket_id: Uuid,
    revision_no: Option<i64>,
    data: &Value,
) -> Option<String> {
    if kind == "ticket.commented" {
        let comment = data.get("commentId")?.as_str()?;
        return clients.get(comment).cloned();
    }
    // `is_link_activity` also lists kinds that carry no field changes; creation still has its
    // own revision, the others only point at whatever revision was current.
    if is_link_activity(kind) && kind != "ticket.created" {
        return None;
    }
    clients
        .get(&format!("{ticket_id}:{}", revision_no?))
        .cloned()
}

fn activity_keys(rows: &[sqlx::postgres::PgRow]) -> (Vec<(Uuid, i64)>, Vec<String>) {
    let mut revisions = Vec::new();
    let mut comments = Vec::new();
    for row in rows {
        let kind: String = row.get("activity_kind");
        if kind == "ticket.commented" {
            let data: Value = row.get("data");
            if let Some(id) = data.get("commentId").and_then(Value::as_str) {
                comments.push(id.to_string());
            }
        } else if (!is_link_activity(&kind) || kind == "ticket.created")
            && let Some(no) = row.get::<Option<i64>, _>("ticket_revision_no")
        {
            revisions.push((row.get::<Uuid, _>("ticket_id"), no));
        }
    }
    (revisions, comments)
}

pub async fn ticket_activity(
    pool: &PgPool,
    actor_id: Uuid,
    ticket_id: Uuid,
    limit: Option<i64>,
    before: Option<&str>,
) -> Result<ActivityPage, CoreError> {
    let limit = paging::clamp_limit(limit, PAGE_DEFAULT, PAGE_MAX);
    let cursor = before.map(paging::decode_cursor).transpose()?;
    let mut tx = pool.begin().await?;
    let access = read_access(&mut tx, actor_id, ticket_id).await?;
    let mut rows = sqlx::query(
        "SELECT a.id,a.ticket_id,a.ticket_revision_no,a.actor_user_id,u.display_name AS actor_name,a.activity_kind,a.data,a.occurred_at
         FROM penfii.activity_entries a LEFT JOIN penfii.users u ON u.id=a.actor_user_id
         WHERE a.ticket_id=$1 AND ($2::timestamptz IS NULL OR (a.occurred_at,a.id) < ($2::timestamptz,$3::uuid))
         ORDER BY a.occurred_at DESC,a.id DESC LIMIT $4",
    )
    .bind(ticket_id)
    .bind(cursor.map(|c| c.0))
    .bind(cursor.map(|c| c.1))
    .bind(limit + 1)
    .fetch_all(&mut *tx)
    .await?;
    let more = rows.len() as i64 > limit;
    rows.truncate(limit as usize);
    let revisions = with_previous(
        rows.iter()
            .filter(|row| {
                !is_link_activity(&row.get::<String, _>("activity_kind"))
                    && row.get::<Option<i64>, _>("ticket_revision_no").is_some()
            })
            .filter_map(|row| row.get::<Option<i64>, _>("ticket_revision_no")),
    );
    let names = status_names(&mut tx, access.project_id).await?;
    let views = revision_views(&mut tx, ticket_id, &revisions, &names).await?;
    let (revision_keys, comment_keys) = activity_keys(&rows);
    let clients = mcp_clients(&mut tx, &revision_keys, &comment_keys).await?;
    tx.commit().await?;
    let next_cursor = if more {
        rows.last()
            .map(|row| paging::encode_cursor(row.get("occurred_at"), row.get("id")))
    } else {
        None
    };
    let items = rows
        .into_iter()
        .map(|row| {
            let kind: String = row.get("activity_kind");
            let revision_no: Option<i64> = row.get("ticket_revision_no");
            let changes = match (kind.as_str(), revision_no) {
                (kind, _) if is_link_activity(kind) => Vec::new(),
                (_, None) => Vec::new(),
                (_, Some(n)) => match (views.get(&(n - 1)), views.get(&n)) {
                    (Some(from), Some(to)) => work_history::diff(from, to),
                    _ => Vec::new(),
                },
            };
            let data: Value = row.get("data");
            let via = activity_via(&clients, &kind, ticket_id, revision_no, &data);
            TicketActivity {
                id: row.get("id"),
                ticket_id: row.get("ticket_id"),
                revision_no,
                activity_kind: kind,
                actor_user_id: row.get("actor_user_id"),
                actor_name: row.get("actor_name"),
                data,
                via,
                changes,
                occurred_at: row.get("occurred_at"),
            }
        })
        .collect();
    Ok(ActivityPage { items, next_cursor })
}

/// Project-wide recent Activity. It carries no field changes; open a Ticket for those.
pub async fn activity(
    pool: &PgPool,
    actor_id: Uuid,
    project_id: Uuid,
) -> Result<Vec<TicketActivity>, CoreError> {
    let mut tx = pool.begin().await?;
    let _ = require_project_role(&mut tx, project_id, actor_id, false).await?;
    let rows=sqlx::query("SELECT a.id,a.ticket_id,a.origin_channel_id,a.ticket_revision_no,a.actor_user_id,u.display_name AS actor_name,a.activity_kind,a.data,a.occurred_at FROM penfii.activity_entries a JOIN penfii.tickets t ON t.id=a.ticket_id LEFT JOIN penfii.users u ON u.id=a.actor_user_id WHERE a.origin_project_id=$1 ORDER BY a.occurred_at DESC,a.id DESC LIMIT 100").bind(project_id).fetch_all(&mut *tx).await?;
    let (revision_keys, comment_keys) = activity_keys(&rows);
    let clients = mcp_clients(&mut tx, &revision_keys, &comment_keys).await?;
    let mut out = Vec::new();
    for row in rows {
        let channel: Option<Uuid> = row.get("origin_channel_id");
        if !channel_allowed(&mut tx, channel, actor_id).await? {
            continue;
        }
        let kind: String = row.get("activity_kind");
        let revision_no: Option<i64> = row.get("ticket_revision_no");
        let data: Value = row.get("data");
        let via = activity_via(&clients, &kind, row.get("ticket_id"), revision_no, &data);
        out.push(TicketActivity {
            id: row.get("id"),
            ticket_id: row.get("ticket_id"),
            revision_no,
            activity_kind: kind,
            actor_user_id: row.get("actor_user_id"),
            actor_name: row.get("actor_name"),
            data,
            via,
            changes: Vec::new(),
            occurred_at: row.get("occurred_at"),
        });
    }
    tx.commit().await?;
    Ok(out)
}

/// History: the revision list. Only revision metadata and the changed field names are
/// returned; a revision's values are opened one at a time with `revision_detail`.
pub async fn revisions(
    pool: &PgPool,
    actor_id: Uuid,
    ticket_id: Uuid,
    limit: Option<i64>,
    before: Option<i64>,
) -> Result<RevisionPage, CoreError> {
    let limit = paging::clamp_limit(limit, PAGE_DEFAULT, PAGE_MAX);
    let mut tx = pool.begin().await?;
    let access = read_access(&mut tx, actor_id, ticket_id).await?;
    let mut rows = sqlx::query(
        "SELECT r.revision_no,r.actor_user_id,u.display_name AS actor_name,r.recorded_at,r.restore_source_revision_no
         FROM penfii.ticket_revisions r LEFT JOIN penfii.users u ON u.id=r.actor_user_id
         WHERE r.ticket_id=$1 AND ($2::bigint IS NULL OR r.revision_no < $2)
         ORDER BY r.revision_no DESC LIMIT $3",
    )
    .bind(ticket_id)
    .bind(before)
    .bind(limit + 1)
    .fetch_all(&mut *tx)
    .await?;
    let more = rows.len() as i64 > limit;
    rows.truncate(limit as usize);
    let numbers = with_previous(rows.iter().map(|row| row.get::<i64, _>("revision_no")));
    let names = status_names(&mut tx, access.project_id).await?;
    let views = revision_views(&mut tx, ticket_id, &numbers, &names).await?;
    let keys: Vec<(Uuid, i64)> = rows
        .iter()
        .map(|row| (ticket_id, row.get::<i64, _>("revision_no")))
        .collect();
    let clients = mcp_clients(&mut tx, &keys, &[]).await?;
    tx.commit().await?;
    let next_before = if more {
        rows.last().map(|row| row.get::<i64, _>("revision_no"))
    } else {
        None
    };
    let items = rows
        .into_iter()
        .map(|row| {
            let revision_no: i64 = row.get("revision_no");
            let changed_fields = match (views.get(&(revision_no - 1)), views.get(&revision_no)) {
                (Some(from), Some(to)) => work_history::diff(from, to)
                    .into_iter()
                    .map(|change| change.field)
                    .collect(),
                _ => Vec::new(),
            };
            RevisionSummary {
                revision_no,
                actor_user_id: row.get("actor_user_id"),
                actor_name: row.get("actor_name"),
                recorded_at: row.get("recorded_at"),
                changed_fields,
                restore_source_revision_no: row.get("restore_source_revision_no"),
                via: clients.get(&format!("{ticket_id}:{revision_no}")).cloned(),
            }
        })
        .collect();
    Ok(RevisionPage { items, next_before })
}

/// One revision opened on demand: what changed in it, and how it differs from the current
/// Ticket, with the fields that could be restored.
pub async fn revision_detail(
    pool: &PgPool,
    actor_id: Uuid,
    ticket_id: Uuid,
    revision_no: i64,
) -> Result<RevisionDetail, CoreError> {
    let mut tx = pool.begin().await?;
    let access = read_access(&mut tx, actor_id, ticket_id).await?;
    let row = sqlx::query(
        "SELECT r.snapshot,r.actor_user_id,u.display_name AS actor_name,r.recorded_at,r.restore_source_revision_no
         FROM penfii.ticket_revisions r LEFT JOIN penfii.users u ON u.id=r.actor_user_id
         WHERE r.ticket_id=$1 AND r.revision_no=$2",
    )
    .bind(ticket_id)
    .bind(revision_no)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(CoreError::NotFound)?;
    let snapshot: Value = row.try_get("snapshot")?;
    let names = status_names(&mut tx, access.project_id).await?;
    let previous = revision_views(&mut tx, ticket_id, &[revision_no - 1], &names).await?;
    let at_revision = work_history::view(&snapshot, &names);
    let changes_from_previous = match previous.get(&(revision_no - 1)) {
        Some(before) => work_history::diff(before, &at_revision),
        None => Vec::new(),
    };
    let current = live_ticket(&mut tx, ticket_id, actor_id).await?;
    let current_snapshot = snapshot_value(&mut tx, &current).await?;
    let changes_to_current =
        work_history::diff(&at_revision, &work_history::view(&current_snapshot, &names));
    let mut restorable = Vec::new();
    for change in &changes_to_current {
        if !RESTORABLE_FIELDS.contains(&change.field.as_str()) {
            continue;
        }
        let reason = restore_blocker(&mut tx, &current, &snapshot, &change.field).await?;
        restorable.push(RestorableField {
            field: change.field.clone(),
            applicable: reason.is_none(),
            reason: reason.map(str::to_owned),
        });
    }
    tx.commit().await?;
    Ok(RevisionDetail {
        revision_no,
        current_revision_no: current.revision_no,
        actor_user_id: row.try_get("actor_user_id")?,
        actor_name: row.try_get("actor_name")?,
        recorded_at: row.try_get("recorded_at")?,
        restore_source_revision_no: row.try_get("restore_source_revision_no")?,
        changes_from_previous,
        changes_to_current,
        restorable,
    })
}
