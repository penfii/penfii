//! Team and Workspace scopes: the same Tickets a Project Board shows, gathered across the Projects
//! the viewer can read, and bulk changes that go through the single-Ticket commands.
//!
//! Nothing here owns data. A scope is a place to look from; the Ticket rows, their permissions and
//! their rules stay where they were.

use penfii_contracts::{
    BulkItemResult, BulkResult, ChildSummary, ScopeDataset, ScopeProject, ScopedTicket, Ticket,
    TicketLabel,
};
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool, Row};
use std::collections::HashMap;
use uuid::Uuid;

use crate::error::CoreError;
use crate::planning;
use crate::platform::{self, ChangeNotice};
use crate::project_role::{channel_allowed, project_role};
use crate::work::{self, TICKET_COLUMNS, TICKET_FROM, TicketRow, TicketUpdate};

/// A scope never returns more than this many Tickets; `truncated` says so.
pub const SCOPE_TICKET_LIMIT: i64 = 1000;
/// A bulk change touches at most this many Tickets.
pub const BULK_LIMIT: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeTarget {
    Team { workspace_id: Uuid, team_id: Uuid },
    Workspace(Uuid),
}

/// Which of the viewer's Tickets a Workspace scope starts from (My Work). `All` applies no lens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lens {
    All,
    Assigned,
    Created,
    Watching,
}

pub async fn scope_dataset(
    pool: &PgPool,
    actor_id: Uuid,
    target: ScopeTarget,
    lens: Lens,
    only_projects: &[Uuid],
) -> Result<ScopeDataset, CoreError> {
    let mut tx = pool.begin().await?;
    let workspace_id = match target {
        ScopeTarget::Team { workspace_id, .. } | ScopeTarget::Workspace(workspace_id) => {
            workspace_id
        }
    };
    crate::access::require_member(&mut tx, workspace_id, actor_id).await?;
    let project_rows = match target {
        ScopeTarget::Team { team_id, .. } => {
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM penfii.teams WHERE id=$1 AND workspace_id=$2)",
            )
            .bind(team_id)
            .bind(workspace_id)
            .fetch_one(&mut *tx)
            .await?;
            if !exists {
                return Err(CoreError::NotFound);
            }
            sqlx::query(
                "SELECT p.id,p.name FROM penfii.projects p
                 JOIN penfii.project_teams pt ON pt.project_id=p.id AND pt.team_id=$2
                 WHERE p.workspace_id=$1 AND p.archived_at IS NULL ORDER BY p.name,p.id",
            )
            .bind(workspace_id)
            .bind(team_id)
            .fetch_all(&mut *tx)
            .await?
        }
        ScopeTarget::Workspace(_) => {
            sqlx::query(
                "SELECT id,name FROM penfii.projects WHERE workspace_id=$1 AND archived_at IS NULL ORDER BY name,id",
            )
            .bind(workspace_id)
            .fetch_all(&mut *tx)
            .await?
        }
    };
    let mut projects = Vec::new();
    for row in project_rows {
        let id: Uuid = row.try_get("id")?;
        // A Project the viewer cannot read is not part of the scope, and leaves no trace in it.
        let (_, role) = project_role(&mut tx, id, actor_id).await?;
        if let Some(role) = role {
            projects.push(ScopeProject {
                id,
                name: row.try_get("name")?,
                can_edit: crate::project_role::can_collaborate(&role),
            });
        }
    }
    // Asking for some Projects narrows the Tickets before the cap, so a big Project cannot crowd out a
    // small one. The list of Projects stays whole so the person can widen the scope again.
    let project_ids: Vec<Uuid> = projects
        .iter()
        .map(|p| p.id)
        .filter(|id| only_projects.is_empty() || only_projects.contains(id))
        .collect();
    let lens_sql = match lens {
        Lens::All => "",
        Lens::Assigned => " AND t.assignee_id=$2",
        Lens::Created => " AND t.created_by=$2",
        Lens::Watching => {
            " AND EXISTS(SELECT 1 FROM penfii.subscriptions s WHERE s.user_id=$2 AND s.target_kind='TICKET' AND s.target_id=t.id AND s.mode IN ('AUTO','EXPLICIT'))"
        }
    };
    // When a scope holds more than the cap, the most recently updated Tickets are the ones kept;
    // among those the order is the Project's shared order.
    let rows = sqlx::query_as::<_, TicketRow>(&format!(
        "WITH picked AS (
            SELECT t.id FROM penfii.tickets t
            WHERE t.project_id=ANY($1){lens_sql}
              AND t.archived_at IS NULL AND t.trashed_at IS NULL AND t.purged_at IS NULL
            ORDER BY t.updated_at DESC, t.id LIMIT {limit}
         )
         SELECT {TICKET_COLUMNS} FROM {TICKET_FROM}
         LEFT JOIN penfii.project_ticket_order o ON o.ticket_id=t.id
         WHERE t.id IN (SELECT id FROM picked)
         ORDER BY t.project_id, o.sort_key NULLS LAST, t.created_at, t.id",
        limit = SCOPE_TICKET_LIMIT + 1
    ))
    .bind(&project_ids)
    .bind(actor_id)
    .fetch_all(&mut *tx)
    .await?;
    let truncated = rows.len() as i64 > SCOPE_TICKET_LIMIT;
    let mut tickets: Vec<Ticket> = rows.into_iter().map(work::ticket).collect();
    if truncated
        && let Some(oldest) = tickets
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                a.updated_at
                    .cmp(&b.updated_at)
                    .then_with(|| b.id.cmp(&a.id))
            })
            .map(|(index, _)| index)
    {
        // The extra row only proved there was more; the least recently updated one is dropped.
        tickets.remove(oldest);
    }
    let mut access: HashMap<Uuid, bool> = HashMap::new();
    for item in &tickets {
        if let Some(channel_id) = item.channel_id
            && let std::collections::hash_map::Entry::Vacant(entry) = access.entry(channel_id)
        {
            entry.insert(channel_allowed(&mut tx, Some(channel_id), actor_id).await?);
        }
    }
    tickets.retain(|item| {
        item.channel_id
            .is_none_or(|channel| access.get(&channel).copied().unwrap_or(false))
    });
    // From here on nothing decides who may see what: `projects` holds only Projects the viewer can
    // read and `tickets` only Tickets whose channel they may open. What follows describes them.
    let channel_ids: Vec<Uuid> = access.keys().copied().collect();
    let tickets = describe_tickets(&mut tx, actor_id, &projects, tickets, &channel_ids).await?;
    tx.commit().await?;
    Ok(ScopeDataset {
        projects,
        tickets,
        truncated,
    })
}

/// Adds what a scope row shows beside the Ticket: labels, planning fields, State and channel names,
/// the viewer's watch flag and the children counts. Reads only; `tickets` has already been cut down
/// to what the viewer may see, and a child count counts only those.
async fn describe_tickets(
    tx: &mut PgConnection,
    actor_id: Uuid,
    projects: &[ScopeProject],
    mut tickets: Vec<Ticket>,
    channel_ids: &[Uuid],
) -> Result<Vec<ScopedTicket>, CoreError> {
    // Labels in one query.
    let ids: Vec<Uuid> = tickets.iter().map(|t| t.id).collect();
    let label_rows = sqlx::query(
        "SELECT tl.ticket_id,l.id,l.name,l.color FROM penfii.ticket_labels tl
         JOIN penfii.labels l ON l.id=tl.label_id AND l.retired_at IS NULL
         WHERE tl.ticket_id=ANY($1) ORDER BY l.name,l.id",
    )
    .bind(&ids)
    .fetch_all(&mut *tx)
    .await?;
    let mut labels: HashMap<Uuid, Vec<TicketLabel>> = HashMap::new();
    for row in label_rows {
        labels
            .entry(row.try_get("ticket_id")?)
            .or_default()
            .push(TicketLabel {
                id: row.try_get("id")?,
                name: row.try_get("name")?,
                color: row.try_get("color")?,
            });
    }
    for item in &mut tickets {
        item.labels = labels.remove(&item.id).unwrap_or_default();
    }
    planning::attach_planning(tx, &mut tickets, actor_id).await?;
    // State names and categories, channel names.
    let status_ids: Vec<Uuid> = tickets.iter().map(|t| t.status_id).collect();
    let state_rows =
        sqlx::query("SELECT id,name,category FROM penfii.workflow_statuses WHERE id=ANY($1)")
            .bind(&status_ids)
            .fetch_all(&mut *tx)
            .await?;
    let mut states: HashMap<Uuid, (String, String)> = HashMap::new();
    for row in state_rows {
        states.insert(
            row.try_get("id")?,
            (row.try_get("name")?, row.try_get("category")?),
        );
    }
    let watched: std::collections::HashSet<Uuid> = sqlx::query_scalar(
        "SELECT target_id FROM penfii.subscriptions WHERE user_id=$1 AND target_kind='TICKET' AND mode IN ('AUTO','EXPLICIT') AND target_id=ANY($2)",
    )
    .bind(actor_id)
    .bind(&ids)
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .collect();
    let channel_rows = sqlx::query("SELECT id,name FROM penfii.channels WHERE id=ANY($1)")
        .bind(channel_ids)
        .fetch_all(&mut *tx)
        .await?;
    let mut channels: HashMap<Uuid, String> = HashMap::new();
    for row in channel_rows {
        channels.insert(row.try_get("id")?, row.try_get("name")?);
    }
    // Child counts come from the Tickets the viewer can see in this scope.
    let mut summaries: HashMap<Uuid, ChildSummary> = HashMap::new();
    for item in &tickets {
        let Some(parent) = item.parent_id else {
            continue;
        };
        let summary = summaries.entry(parent).or_default();
        summary.total += 1;
        match states.get(&item.status_id).map(|s| s.1.as_str()) {
            Some("DONE") => summary.done += 1,
            Some("CANCELED") => summary.canceled += 1,
            _ => {}
        }
    }
    let names: HashMap<Uuid, &str> = projects.iter().map(|p| (p.id, p.name.as_str())).collect();
    let tickets = tickets
        .into_iter()
        .map(|mut ticket| {
            if let Some(summary) = summaries.remove(&ticket.id) {
                ticket.child_summary = summary;
            }
            let (state_name, state_category) =
                states.get(&ticket.status_id).cloned().unwrap_or_default();
            ScopedTicket {
                watching: watched.contains(&ticket.id),
                project_name: names
                    .get(&ticket.project_id)
                    .copied()
                    .unwrap_or_default()
                    .to_owned(),
                channel_name: ticket.channel_id.and_then(|c| channels.get(&c).cloned()),
                state_name,
                state_category,
                ticket,
            }
        })
        .collect();
    Ok(tickets)
}

// ---------------------------------------------------------------------------------------------
// Bulk changes
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum BulkChange {
    Status(Uuid),
    Assignee(Option<Uuid>),
    Priority(Option<String>),
    AddLabels(Vec<Uuid>),
    RemoveLabels(Vec<Uuid>),
    Sprint(Option<Uuid>),
    AddMilestones(Vec<Uuid>),
    RemoveMilestones(Vec<Uuid>),
}

impl BulkChange {
    fn to_json(&self) -> Value {
        match self {
            Self::Status(id) => json!({"type":"STATUS","statusId":id}),
            Self::Assignee(id) => json!({"type":"ASSIGNEE","assigneeId":id}),
            Self::Priority(p) => json!({"type":"PRIORITY","priority":p}),
            Self::AddLabels(ids) => json!({"type":"ADD_LABELS","labelIds":ids}),
            Self::RemoveLabels(ids) => json!({"type":"REMOVE_LABELS","labelIds":ids}),
            Self::Sprint(id) => json!({"type":"SPRINT","sprintId":id}),
            Self::AddMilestones(ids) => json!({"type":"ADD_MILESTONES","milestoneIds":ids}),
            Self::RemoveMilestones(ids) => json!({"type":"REMOVE_MILESTONES","milestoneIds":ids}),
        }
    }
}

fn failure(error: CoreError) -> (String, String) {
    match error {
        CoreError::NotFound => ("not_found".into(), "the Ticket is not available".into()),
        CoreError::Unauthenticated => ("unauthenticated".into(), "sign in again".into()),
        CoreError::Validation(message) => ("validation".into(), message),
        CoreError::Forbidden(message) => ("forbidden".into(), message),
        CoreError::Conflict { code, message, .. } | CoreError::Rule { code, message } => {
            (code.into(), message)
        }
        CoreError::ReauthRequired { .. } => {
            ("reauth_required".into(), "sign in again to continue".into())
        }
        CoreError::RateLimited { .. } => ("rate_limited".into(), "too many attempts".into()),
        CoreError::Internal(message) => ("internal".into(), message),
    }
}

async fn change_one(
    pool: &PgPool,
    actor_id: Uuid,
    ticket_id: Uuid,
    change: &BulkChange,
    key: &str,
) -> Result<(Ticket, Option<ChangeNotice>), CoreError> {
    let current = work::get_ticket(pool, actor_id, ticket_id).await?;
    match change {
        BulkChange::Status(status_id) => {
            work::change_status(
                pool,
                actor_id,
                ticket_id,
                current.revision_no,
                *status_id,
                key,
            )
            .await
        }
        BulkChange::Assignee(assignee) => {
            let update = TicketUpdate {
                title: None,
                description: None,
                assignee_id: Some(*assignee),
                priority: None,
                due_on: None,
                label_ids: None,
                parent_id: None,
                type_id: None,
                ..Default::default()
            };
            work::update_ticket(pool, actor_id, ticket_id, current.revision_no, update, key).await
        }
        BulkChange::Priority(priority) => {
            let update = TicketUpdate {
                title: None,
                description: None,
                assignee_id: None,
                priority: Some(priority.clone()),
                due_on: None,
                label_ids: None,
                parent_id: None,
                type_id: None,
                ..Default::default()
            };
            work::update_ticket(pool, actor_id, ticket_id, current.revision_no, update, key).await
        }
        BulkChange::AddLabels(ids) | BulkChange::RemoveLabels(ids) => {
            let mut labels: Vec<Uuid> = current.labels.iter().map(|l| l.id).collect();
            if matches!(change, BulkChange::AddLabels(_)) {
                for id in ids {
                    if !labels.contains(id) {
                        labels.push(*id);
                    }
                }
            } else {
                labels.retain(|id| !ids.contains(id));
            }
            let update = TicketUpdate {
                title: None,
                description: None,
                assignee_id: None,
                priority: None,
                due_on: None,
                label_ids: Some(labels),
                parent_id: None,
                type_id: None,
                ..Default::default()
            };
            work::update_ticket(pool, actor_id, ticket_id, current.revision_no, update, key).await
        }
        BulkChange::Sprint(sprint) => {
            planning::assign_sprint(pool, actor_id, ticket_id, *sprint, key).await
        }
        BulkChange::AddMilestones(ids) | BulkChange::RemoveMilestones(ids) => {
            let mut milestones: Vec<Uuid> = current.milestones.iter().map(|m| m.id).collect();
            if matches!(change, BulkChange::AddMilestones(_)) {
                for id in ids {
                    if !milestones.contains(id) {
                        milestones.push(*id);
                    }
                }
            } else {
                milestones.retain(|id| !ids.contains(id));
            }
            planning::set_milestones(pool, actor_id, ticket_id, milestones, key).await
        }
    }
}

/// Applies one change to up to [`BULK_LIMIT`] Tickets. Each Ticket goes through the same command a
/// single edit uses, in its own transaction, so permissions, Workflow transitions, dependency
/// blocks and revision checks are never bypassed; the result lists every Ticket. A failure on one
/// Ticket never hides or undoes the others.
pub async fn bulk_change(
    pool: &PgPool,
    actor_id: Uuid,
    workspace_id: Uuid,
    ticket_ids: Vec<Uuid>,
    change: BulkChange,
    idem: &str,
) -> Result<(BulkResult, Vec<ChangeNotice>), CoreError> {
    let mut seen = std::collections::HashSet::new();
    let ticket_ids: Vec<Uuid> = ticket_ids
        .into_iter()
        .filter(|id| seen.insert(*id))
        .collect();
    if ticket_ids.is_empty() {
        return Err(CoreError::validation("select at least one Ticket"));
    }
    if ticket_ids.len() > BULK_LIMIT {
        return Err(CoreError::validation(format!(
            "a bulk change accepts at most {BULK_LIMIT} Tickets"
        )));
    }
    {
        let mut conn = pool.acquire().await?;
        crate::access::require_member(&mut conn, workspace_id, actor_id).await?;
    }
    let change_json = change.to_json();
    let mut items = Vec::new();
    let mut notices = Vec::new();
    for ticket_id in ticket_ids {
        // One key for the Ticket's own command, another for the record of this item's result.
        let item_key = format!("bulk:{idem}:{ticket_id}");
        let receipt_key = format!("bulk-item:{idem}:{ticket_id}");
        let digest = platform::fingerprint(
            "bulk.item",
            &json!({"ticketId":ticket_id,"change":change_json}),
        );
        // A repeated request returns what the first one recorded for this Ticket. This is not
        // `platform::replay`/`finish`: each item's receipt is looked up in a transaction of its own that
        // is thrown away when fresh, and written in another one after the item has run (success or
        // failure), so a failed item is recorded too and a retry does not redo it.
        {
            let mut tx = pool.begin().await?;
            if let platform::Claim::Replayed { body, .. } = platform::claim(
                &mut tx,
                workspace_id,
                &platform::actor_key(actor_id),
                &receipt_key,
                "bulk.item",
                &digest,
            )
            .await?
            {
                tx.commit().await?;
                items.push(
                    serde_json::from_value::<BulkItemResult>(body)
                        .map_err(|e| CoreError::Internal(e.to_string()))?,
                );
                continue;
            }
            tx.rollback().await?;
        }
        let result = change_one(pool, actor_id, ticket_id, &change, &item_key).await;
        let item = match result {
            Ok((ticket, notice)) => {
                notices.extend(notice);
                BulkItemResult {
                    ticket_id,
                    key: Some(ticket.key),
                    ok: true,
                    code: None,
                    message: None,
                }
            }
            Err(error) => {
                let (code, message) = failure(error);
                BulkItemResult {
                    ticket_id,
                    key: None,
                    ok: false,
                    code: Some(code),
                    message: Some(message),
                }
            }
        };
        let mut tx = pool.begin().await?;
        if let platform::Claim::Fresh = platform::claim(
            &mut tx,
            workspace_id,
            &platform::actor_key(actor_id),
            &receipt_key,
            "bulk.item",
            &digest,
        )
        .await?
        {
            platform::complete(
                &mut tx,
                workspace_id,
                &platform::actor_key(actor_id),
                &receipt_key,
                200,
                &serde_json::to_value(&item).map_err(|e| CoreError::Internal(e.to_string()))?,
            )
            .await?;
        }
        tx.commit().await?;
        items.push(item);
    }
    let succeeded = items.iter().filter(|i| i.ok).count() as i64;
    Ok((
        BulkResult {
            total: items.len() as i64,
            succeeded,
            failed: items.len() as i64 - succeeded,
            items,
        },
        notices,
    ))
}
