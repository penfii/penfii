//! The Gantt list of one Project for the person asking: its Tickets with their planned schedules
//! and the blocking dependencies between the Tickets they can see.

use chrono::{DateTime, Utc};
use penfii_contracts::{
    GanttCalendar, GanttEdge, GanttItem, ProjectSchedule, TicketRef, TicketSchedule,
};
use sqlx::{PgConnection, PgPool, Row};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use crate::calendar;
use crate::error::CoreError;
use crate::planning::visible;
use crate::project_role::require_project_role;

const PAGE_DEFAULT: i64 = 300;
const PAGE_MAX: i64 = 1000;

struct Facts {
    id: Uuid,
    key: String,
    title: String,
    kind: String,
    parent: Option<Uuid>,
    channel: Option<Uuid>,
    category: String,
    revision: i64,
    assignee: Option<String>,
    sequence: i64,
    start: Option<DateTime<Utc>>,
    duration: Option<i64>,
    milestone: bool,
    has_schedule: bool,
}

struct EdgeFacts {
    id: Uuid,
    pred: Uuid,
    succ: Uuid,
    waived: bool,
}

/// What the person can see of the Project: its live Tickets and the blocking dependencies between them.
struct Visible {
    tickets: Vec<Facts>,
    edges: Vec<EdgeFacts>,
}

async fn load_visible(
    conn: &mut PgConnection,
    actor_id: Uuid,
    project_id: Uuid,
) -> Result<Visible, CoreError> {
    let rows = sqlx::query(
        "SELECT t.id, t.public_key, t.title, t.hierarchy_kind, t.parent_id, t.channel_id, s.category,
                t.revision_no, (SELECT u.display_name FROM penfii.users u WHERE u.id=t.assignee_id) AS assignee,
                t.project_sequence, ts.planned_start_at, ts.duration_minutes,
                ts.is_milestone, (ts.ticket_id IS NOT NULL) AS has_schedule
         FROM penfii.tickets t
         JOIN penfii.workflow_statuses s ON s.id=t.status_id
         LEFT JOIN penfii.ticket_schedules ts ON ts.ticket_id=t.id
         WHERE t.project_id=$1 AND t.archived_at IS NULL AND t.trashed_at IS NULL AND t.purged_at IS NULL
         ORDER BY t.project_sequence, t.id",
    )
    .bind(project_id)
    .fetch_all(&mut *conn)
    .await?;
    let mut access = HashMap::new();
    let mut tickets = Vec::new();
    for row in rows {
        let channel: Option<Uuid> = row.try_get("channel_id")?;
        if !visible(conn, &mut access, channel, actor_id).await? {
            continue;
        }
        tickets.push(Facts {
            id: row.try_get("id")?,
            key: row.try_get("public_key")?,
            title: row.try_get("title")?,
            kind: row.try_get("hierarchy_kind")?,
            parent: row.try_get("parent_id")?,
            channel,
            category: row.try_get("category")?,
            revision: row.try_get("revision_no")?,
            assignee: row.try_get("assignee")?,
            sequence: row.try_get("project_sequence")?,
            start: row.try_get("planned_start_at")?,
            duration: row.try_get("duration_minutes")?,
            milestone: row
                .try_get::<Option<bool>, _>("is_milestone")?
                .unwrap_or(false),
            has_schedule: row.try_get("has_schedule")?,
        });
    }
    let ids: HashSet<Uuid> = tickets.iter().map(|t| t.id).collect();
    let edges = sqlx::query("SELECT id, predecessor_id, successor_id, (waived_at IS NOT NULL) AS waived FROM penfii.ticket_dependencies WHERE project_id=$1 AND kind='BLOCKS' ORDER BY id")
        .bind(project_id)
        .fetch_all(&mut *conn)
        .await?
        .into_iter()
        .map(|row| {
            Ok(EdgeFacts { id: row.try_get("id")?, pred: row.try_get("predecessor_id")?, succ: row.try_get("successor_id")?, waived: row.try_get("waived")? })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()?
        .into_iter()
        .filter(|edge| ids.contains(&edge.pred) && ids.contains(&edge.succ))
        .collect();
    Ok(Visible { tickets, edges })
}

/// The Tickets of the Project with their schedules for the Gantt, and the blocking dependencies
/// between Tickets the person can see. Paged by Ticket number; `channel` narrows the Tickets.
pub async fn schedule(
    pool: &PgPool,
    actor_id: Uuid,
    project_id: Uuid,
    channel: Option<Uuid>,
    cursor: Option<&str>,
    limit: Option<i64>,
) -> Result<ProjectSchedule, CoreError> {
    let limit = crate::paging::clamp_limit(limit, PAGE_DEFAULT, PAGE_MAX);
    let after: i64 = match cursor {
        Some(text) => text
            .parse()
            .map_err(|_| CoreError::validation("invalid cursor"))?,
        None => 0,
    };
    let mut tx = pool.begin().await?;
    let (_, role) = require_project_role(&mut tx, project_id, actor_id, false).await?;
    let calendar = calendar::load(&mut tx, project_id).await?;
    let visible = load_visible(&mut tx, actor_id, project_id).await?;
    let page: Vec<&Facts> = visible
        .tickets
        .iter()
        .filter(|t| t.sequence > after && channel.is_none_or(|c| t.channel == Some(c)))
        .take(limit as usize + 1)
        .collect();
    let more = page.len() as i64 > limit;
    let page: Vec<&Facts> = page.into_iter().take(limit as usize).collect();
    let next_cursor = if more {
        page.last().map(|t| t.sequence.to_string())
    } else {
        None
    };
    // The planned end of each schedule that has a start and a duration.
    let ends: Vec<(usize, DateTime<Utc>, i64)> = page
        .iter()
        .enumerate()
        .filter_map(|(i, t)| {
            let start = t.start?;
            let minutes = if t.milestone { Some(0) } else { t.duration };
            minutes.map(|m| (i, start, m))
        })
        .collect();
    let computed = calendar::planned_ends(
        &mut tx,
        project_id,
        &ends.iter().map(|(_, s, m)| (*s, *m)).collect::<Vec<_>>(),
    )
    .await?;
    tx.commit().await?;
    let mut end_of: HashMap<usize, DateTime<Utc>> = HashMap::new();
    for ((index, _, _), end) in ends.iter().zip(computed) {
        if let Some(end) = end {
            end_of.insert(*index, end);
        }
    }
    let page_ids: HashSet<Uuid> = page.iter().map(|t| t.id).collect();
    let categories: HashMap<Uuid, &str> = visible
        .tickets
        .iter()
        .map(|t| (t.id, t.category.as_str()))
        .collect();
    let items = page
        .iter()
        .enumerate()
        .map(|(i, t)| GanttItem {
            ticket: TicketRef {
                id: t.id,
                key: t.key.clone(),
                title: t.title.clone(),
                hierarchy_kind: t.kind.clone(),
            },
            parent_id: t.parent,
            channel_id: t.channel,
            status_category: t.category.clone(),
            assignee_name: t.assignee.clone(),
            revision_no: t.revision,
            schedule: t.has_schedule.then(|| TicketSchedule {
                planned_start_at: t.start,
                duration_minutes: t.duration,
                is_schedule_milestone: t.milestone,
                planned_end_at: end_of.get(&i).copied(),
            }),
        })
        .collect();
    let edges = visible
        .edges
        .iter()
        .filter(|e| page_ids.contains(&e.pred) || page_ids.contains(&e.succ))
        .map(|e| GanttEdge {
            id: e.id,
            predecessor_id: e.pred,
            successor_id: e.succ,
            waived: e.waived,
            predecessor_category: categories
                .get(&e.pred)
                .copied()
                .unwrap_or("NOT_STARTED")
                .to_owned(),
        })
        .collect();
    Ok(ProjectSchedule {
        calendar: calendar.map(|c| GanttCalendar {
            version: c.version,
            time_zone: c.time_zone,
        }),
        can_edit: crate::project_role::can_collaborate(&role),
        items,
        edges,
        next_cursor,
    })
}
