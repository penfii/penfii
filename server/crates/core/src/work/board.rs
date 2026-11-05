//! The project Board and the Labels, members and Ticket types it is built from.

use penfii_contracts::{
    BoardColumn, ChildSummary, ProjectBoard, ProjectMember, Ticket, TicketLabel, TicketType,
};
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use uuid::Uuid;

use super::states::{StateRow, state};
use super::{TICKET_COLUMNS, TICKET_FROM, TicketRow, attach_labels, ticket};
use crate::error::CoreError;
use crate::planning;
use crate::project_role::{can_collaborate, can_manage, channel_allowed, require_project_role};

pub async fn board(
    pool: &PgPool,
    project_id: Uuid,
    actor_id: Uuid,
) -> Result<ProjectBoard, CoreError> {
    let mut tx = pool.begin().await?;
    let (_, role) = require_project_role(&mut tx, project_id, actor_id, false).await?;
    let workflow_version: i64 =
        sqlx::query_scalar("SELECT version FROM penfii.workflows WHERE project_id=$1")
            .bind(project_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(CoreError::NotFound)?;
    let states = sqlx::query_as::<_, StateRow>("SELECT id,project_id,name,category,position,is_initial,wip_limit FROM penfii.workflow_statuses WHERE project_id=$1 AND retired_at IS NULL ORDER BY position,id").bind(project_id).fetch_all(&mut *tx).await?.into_iter().map(state).collect::<Vec<_>>();
    let rows = sqlx::query_as::<_, TicketRow>(
        &format!(
        "SELECT {TICKET_COLUMNS} FROM {TICKET_FROM}
         LEFT JOIN penfii.project_ticket_order o ON o.ticket_id=t.id
         WHERE t.project_id=$1 AND t.archived_at IS NULL AND t.trashed_at IS NULL AND t.purged_at IS NULL
         ORDER BY o.sort_key NULLS LAST,t.created_at,t.id"),
    ).bind(project_id).fetch_all(&mut *tx).await?;
    let mut tickets = rows.into_iter().map(ticket).collect::<Vec<_>>();
    let mut channel_access = HashMap::new();
    for item in &tickets {
        if let Some(channel_id) = item.channel_id
            && let std::collections::hash_map::Entry::Vacant(entry) =
                channel_access.entry(channel_id)
        {
            entry.insert(channel_allowed(&mut tx, Some(channel_id), actor_id).await?);
        }
    }
    tickets.retain(|item| {
        item.channel_id
            .is_none_or(|channel_id| channel_access.get(&channel_id).copied().unwrap_or(false))
    });
    for item in &mut tickets {
        attach_labels(&mut tx, item).await?;
    }
    planning::attach_planning(&mut tx, &mut tickets, actor_id).await?;
    // Child counts come from the tickets this viewer can already see.
    let categories: HashMap<Uuid, &str> =
        states.iter().map(|s| (s.id, s.category.as_str())).collect();
    let mut summaries: HashMap<Uuid, ChildSummary> = HashMap::new();
    for item in &tickets {
        let Some(parent_id) = item.parent_id else {
            continue;
        };
        let summary = summaries.entry(parent_id).or_default();
        summary.total += 1;
        match categories.get(&item.status_id).copied() {
            Some("DONE") => summary.done += 1,
            Some("CANCELED") => summary.canceled += 1,
            _ => {}
        }
    }
    for item in &mut tickets {
        if let Some(summary) = summaries.remove(&item.id) {
            item.child_summary = summary;
        }
    }
    let columns = states
        .iter()
        .map(|s| {
            let in_column: Vec<Ticket> = tickets
                .iter()
                .filter(|t| t.status_id == s.id)
                .cloned()
                .collect();
            BoardColumn {
                id: s.id,
                name: s.name.clone(),
                category: s.category.clone(),
                position: s.position,
                state_id: s.id,
                state_ids: vec![s.id],
                ticket_count: in_column.len() as i64,
                wip_limit: s.wip_limit,
                tickets: in_column,
            }
        })
        .collect();
    let estimate_unit: String =
        sqlx::query_scalar("SELECT estimate_unit FROM penfii.projects WHERE id=$1")
            .bind(project_id)
            .fetch_one(&mut *tx)
            .await?;
    tx.commit().await?;
    Ok(ProjectBoard {
        project_id,
        workflow_version,
        estimate_unit,
        can_edit: can_collaborate(&role),
        can_manage_workflow: can_manage(&role),
        states,
        columns,
    })
}

pub async fn project_labels(
    pool: &PgPool,
    actor_id: Uuid,
    project_id: Uuid,
) -> Result<Vec<TicketLabel>, CoreError> {
    let mut tx = pool.begin().await?;
    let _ = require_project_role(&mut tx, project_id, actor_id, false).await?;
    let rows = sqlx::query("SELECT id,name,color FROM penfii.labels WHERE project_id=$1 AND retired_at IS NULL ORDER BY name,id")
        .bind(project_id)
        .fetch_all(&mut *tx)
        .await?;
    let labels = rows
        .into_iter()
        .map(|row| TicketLabel {
            id: row.get("id"),
            name: row.get("name"),
            color: row.get("color"),
        })
        .collect();
    tx.commit().await?;
    Ok(labels)
}

pub async fn create_project_label(
    pool: &PgPool,
    actor_id: Uuid,
    project_id: Uuid,
    name: &str,
    color: Option<String>,
) -> Result<TicketLabel, CoreError> {
    let name = name.trim();
    if name.is_empty() || name.len() > 50 {
        return Err(CoreError::validation("label name must be 1..50 characters"));
    }
    let mut tx = pool.begin().await?;
    require_project_role(&mut tx, project_id, actor_id, true).await?;
    let id = Uuid::new_v4();
    let row = sqlx::query("INSERT INTO penfii.labels(id,project_id,name,color) VALUES($1,$2,$3,$4) RETURNING id,name,color")
        .bind(id).bind(project_id).bind(name).bind(color)
        .fetch_one(&mut *tx).await?;
    let label = TicketLabel {
        id: row.get("id"),
        name: row.get("name"),
        color: row.get("color"),
    };
    tx.commit().await?;
    Ok(label)
}

pub async fn project_members(
    pool: &PgPool,
    actor_id: Uuid,
    project_id: Uuid,
) -> Result<Vec<ProjectMember>, CoreError> {
    let mut tx = pool.begin().await?;
    let (workspace, _) = require_project_role(&mut tx, project_id, actor_id, false).await?;
    // The people work can be given to: the Project's participants.
    let rows = sqlx::query("SELECT u.id,u.display_name FROM penfii.workspace_members wm JOIN penfii.users u ON u.id=wm.user_id WHERE wm.workspace_id=$1 AND wm.state='ACTIVE' AND penfii.in_share_scope($2,NULL,u.id) ORDER BY u.display_name,u.id")
        .bind(workspace)
        .bind(project_id)
        .fetch_all(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(rows
        .into_iter()
        .map(|row| ProjectMember {
            user_id: row.get("id"),
            display_name: row.get("display_name"),
        })
        .collect())
}

pub async fn project_ticket_types(
    pool: &PgPool,
    actor_id: Uuid,
    project_id: Uuid,
) -> Result<Vec<TicketType>, CoreError> {
    let mut tx = pool.begin().await?;
    let _ = require_project_role(&mut tx, project_id, actor_id, false).await?;
    let rows = sqlx::query("SELECT id,name,color FROM penfii.ticket_types WHERE project_id=$1 AND retired_at IS NULL ORDER BY name,id")
        .bind(project_id)
        .fetch_all(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(rows
        .into_iter()
        .map(|row| TicketType {
            id: row.get("id"),
            name: row.get("name"),
            color: row.get("color"),
        })
        .collect())
}
