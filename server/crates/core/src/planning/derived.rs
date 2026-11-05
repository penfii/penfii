//! The planning fields a Ticket carries: its open blockers, Sprint and Milestones.

use penfii_contracts::{DependencySummary, MilestoneRef, SprintRef, Ticket};
use sqlx::{PgConnection, Row};
use std::collections::HashMap;
use uuid::Uuid;

use crate::error::CoreError;
use crate::project_role::channel_allowed;

/// Open blockers from every predecessor, visible or not: a Ticket with an open blocker cannot be
/// completed. A waived relation and a RELATED link never block. `canceled` counts the blockers
/// whose predecessor was canceled, which can only be cleared by removing or waiving the relation.
pub(crate) async fn open_blockers(
    conn: &mut PgConnection,
    successor_id: Uuid,
) -> Result<(i64, i64), CoreError> {
    let row = sqlx::query(
        "SELECT count(*) AS open, count(*) FILTER (WHERE ps.category='CANCELED') AS canceled
         FROM penfii.ticket_dependencies d
         JOIN penfii.tickets p ON p.id=d.predecessor_id
              AND p.archived_at IS NULL AND p.trashed_at IS NULL AND p.purged_at IS NULL
         JOIN penfii.workflow_statuses ps ON ps.id=p.status_id
         WHERE d.successor_id=$1 AND d.kind='BLOCKS' AND d.waived_at IS NULL AND ps.category<>'DONE'",
    )
    .bind(successor_id)
    .fetch_one(&mut *conn)
    .await?;
    Ok((row.try_get("open")?, row.try_get("canceled")?))
}

/// Fills dependency counts, Sprint and Milestones. Counts include only Tickets the viewer can
/// see, so a hidden Ticket leaves no trace.
pub(crate) async fn attach_planning(
    conn: &mut PgConnection,
    tickets: &mut [Ticket],
    actor_id: Uuid,
) -> Result<(), CoreError> {
    if tickets.is_empty() {
        return Ok(());
    }
    let ids: Vec<Uuid> = tickets.iter().map(|t| t.id).collect();

    let edges = sqlx::query(
        "SELECT d.predecessor_id, d.successor_id, pt.channel_id AS pred_channel,
                st.channel_id AS succ_channel, ps.category AS pred_category,
                (d.waived_at IS NOT NULL) AS waived
         FROM penfii.ticket_dependencies d
         JOIN penfii.tickets pt ON pt.id=d.predecessor_id
              AND pt.archived_at IS NULL AND pt.trashed_at IS NULL AND pt.purged_at IS NULL
         JOIN penfii.tickets st ON st.id=d.successor_id
              AND st.archived_at IS NULL AND st.trashed_at IS NULL AND st.purged_at IS NULL
         JOIN penfii.workflow_statuses ps ON ps.id=pt.status_id
         WHERE d.kind='BLOCKS' AND (d.predecessor_id=ANY($1) OR d.successor_id=ANY($1))",
    )
    .bind(&ids)
    .fetch_all(&mut *conn)
    .await?;
    let mut access: HashMap<Uuid, bool> = HashMap::new();
    let mut summaries: HashMap<Uuid, DependencySummary> = HashMap::new();
    for edge in edges {
        let predecessor: Uuid = edge.try_get("predecessor_id")?;
        let successor: Uuid = edge.try_get("successor_id")?;
        let pred_channel: Option<Uuid> = edge.try_get("pred_channel")?;
        let succ_channel: Option<Uuid> = edge.try_get("succ_channel")?;
        let category: String = edge.try_get("pred_category")?;
        let pred_visible = visible(conn, &mut access, pred_channel, actor_id).await?;
        let succ_visible = visible(conn, &mut access, succ_channel, actor_id).await?;
        if ids.contains(&successor) && pred_visible {
            let entry = summaries.entry(successor).or_default();
            let waived: bool = edge.try_get("waived")?;
            entry.blocked_by += 1;
            if category != "DONE" && !waived {
                entry.open_blockers += 1;
            }
        }
        if ids.contains(&predecessor) && succ_visible {
            summaries.entry(predecessor).or_default().blocks += 1;
        }
    }

    let sprint_rows = sqlx::query(
        "SELECT t.id AS ticket_id, s.id, s.name, s.state, (a.ticket_id<>t.id) AS inherited
         FROM penfii.tickets t
         JOIN penfii.sprint_assignments a ON a.ended_at IS NULL
              AND a.ticket_id=CASE WHEN t.hierarchy_kind='SUBTASK' THEN t.parent_id ELSE t.id END
         JOIN penfii.sprints s ON s.id=a.sprint_id
         WHERE t.id=ANY($1)",
    )
    .bind(&ids)
    .fetch_all(&mut *conn)
    .await?;
    let mut sprints: HashMap<Uuid, SprintRef> = HashMap::new();
    for row in sprint_rows {
        sprints.insert(
            row.try_get("ticket_id")?,
            SprintRef {
                id: row.try_get("id")?,
                name: row.try_get("name")?,
                state: row.try_get("state")?,
                inherited: row.try_get("inherited")?,
            },
        );
    }

    let milestone_rows = sqlx::query(
        "SELECT tm.ticket_id, m.id, m.name, m.state FROM penfii.ticket_milestones tm
         JOIN penfii.milestones m ON m.id=tm.milestone_id
         WHERE tm.ticket_id=ANY($1) ORDER BY m.created_at, m.id",
    )
    .bind(&ids)
    .fetch_all(&mut *conn)
    .await?;
    let mut milestones: HashMap<Uuid, Vec<MilestoneRef>> = HashMap::new();
    for row in milestone_rows {
        milestones
            .entry(row.try_get("ticket_id")?)
            .or_default()
            .push(MilestoneRef {
                id: row.try_get("id")?,
                name: row.try_get("name")?,
                state: row.try_get("state")?,
            });
    }

    let top_rows = sqlx::query(
        "SELECT ticket_id, moved_to_top_at FROM penfii.project_ticket_order
         WHERE ticket_id=ANY($1) AND moved_to_top_at IS NOT NULL",
    )
    .bind(&ids)
    .fetch_all(&mut *conn)
    .await?;
    let mut moved_to_top: HashMap<Uuid, chrono::DateTime<chrono::Utc>> = HashMap::new();
    for row in top_rows {
        moved_to_top.insert(row.try_get("ticket_id")?, row.try_get("moved_to_top_at")?);
    }

    for ticket in tickets {
        ticket.moved_to_top_at = moved_to_top.remove(&ticket.id);
        ticket.dependencies = summaries.remove(&ticket.id).unwrap_or_default();
        ticket.sprint = sprints.remove(&ticket.id);
        ticket.milestones = milestones.remove(&ticket.id).unwrap_or_default();
    }
    Ok(())
}

pub(crate) async fn visible(
    conn: &mut PgConnection,
    cache: &mut HashMap<Uuid, bool>,
    channel: Option<Uuid>,
    actor_id: Uuid,
) -> Result<bool, CoreError> {
    let Some(channel_id) = channel else {
        return Ok(true);
    };
    if let Some(known) = cache.get(&channel_id) {
        return Ok(*known);
    }
    let allowed = channel_allowed(conn, Some(channel_id), actor_id).await?;
    cache.insert(channel_id, allowed);
    Ok(allowed)
}
