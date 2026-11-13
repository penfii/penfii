//! Milestones and the Tickets they hold.

use chrono::{DateTime, NaiveDate, Utc};
use penfii_contracts::{Milestone, Ticket};
use serde_json::{Value, json};
use sqlx::{PgPool, Row};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use super::views::LIST_MAX;
use super::{long_text, require_admin, short_name, version_conflict};
use crate::error::CoreError;
use crate::platform::{self, ChangeNotice};
use crate::project_role::require_ticket_editor;
use crate::work::{attach_derived, attach_planned_end, load_live_ticket, record_change};

pub(super) const MILESTONE_COLUMNS: &str =
    "id,project_id,name,description,target_on,state,reached_at,version";

pub(super) fn milestone_from(row: &sqlx::postgres::PgRow) -> Result<Milestone, CoreError> {
    Ok(Milestone {
        id: row.try_get("id")?,
        project_id: row.try_get("project_id")?,
        name: row.try_get("name")?,
        description: row.try_get("description")?,
        target_on: row.try_get("target_on")?,
        state: row.try_get("state")?,
        reached_at: row.try_get("reached_at")?,
        version: row.try_get("version")?,
    })
}

pub struct MilestoneInput {
    pub name: String,
    pub description: Option<String>,
    pub target_on: Option<NaiveDate>,
}

pub async fn create_milestone(
    pool: &PgPool,
    actor_id: Uuid,
    project_id: Uuid,
    input: MilestoneInput,
    idem: &str,
) -> Result<Milestone, CoreError> {
    let name = short_name(&input.name, "Milestone")?;
    let description = long_text(input.description, "description")?;
    let mut tx = pool.begin().await?;
    let workspace = require_admin(&mut tx, project_id, actor_id).await?;
    let body = json!({"projectId":project_id,"name":name,"description":description,"targetOn":input.target_on});
    if let Some(done) = platform::replay::<Milestone>(
        &mut tx,
        workspace,
        actor_id,
        idem,
        "milestone.create",
        &body,
    )
    .await?
    {
        tx.commit().await?;
        return Ok(done);
    }
    let row = sqlx::query(&format!(
        "INSERT INTO penfii.milestones(id,workspace_id,project_id,name,description,target_on)
         VALUES($1,$2,$3,$4,$5,$6) RETURNING {MILESTONE_COLUMNS}"
    ))
    .bind(Uuid::new_v4())
    .bind(workspace)
    .bind(project_id)
    .bind(&name)
    .bind(&description)
    .bind(input.target_on)
    .fetch_one(&mut *tx)
    .await?;
    let milestone = milestone_from(&row)?;
    platform::finish(&mut tx, workspace, actor_id, idem, 201, &milestone).await?;
    tx.commit().await?;
    Ok(milestone)
}

#[derive(Default)]
pub struct MilestoneUpdate {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub target_on: Option<Option<NaiveDate>>,
    pub state: Option<String>,
}

/// Reaching or canceling a Milestone changes nothing on its Tickets or their Sprints.
pub async fn update_milestone(
    pool: &PgPool,
    actor_id: Uuid,
    milestone_id: Uuid,
    expected_version: i64,
    update: MilestoneUpdate,
) -> Result<Milestone, CoreError> {
    if update
        .state
        .as_deref()
        .is_some_and(|s| !matches!(s, "OPEN" | "REACHED" | "CANCELED"))
    {
        return Err(CoreError::validation(
            "state must be OPEN, REACHED or CANCELED",
        ));
    }
    let mut tx = pool.begin().await?;
    let row = sqlx::query(&format!(
        "SELECT {MILESTONE_COLUMNS} FROM penfii.milestones WHERE id=$1 FOR UPDATE"
    ))
    .bind(milestone_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(CoreError::NotFound)?;
    let project_id: Uuid = row.try_get("project_id")?;
    require_admin(&mut tx, project_id, actor_id).await?;
    let current: i64 = row.try_get("version")?;
    if current != expected_version {
        return Err(version_conflict(current));
    }
    let name = match update.name {
        Some(name) => short_name(&name, "Milestone")?,
        None => row.try_get("name")?,
    };
    let description = match update.description {
        Some(text) => long_text(text, "description")?,
        None => row.try_get("description")?,
    };
    let target_on: Option<NaiveDate> = match update.target_on {
        Some(date) => date,
        None => row.try_get("target_on")?,
    };
    let previous: String = row.try_get("state")?;
    let state: String = update.state.unwrap_or_else(|| previous.clone());
    // The first transition into REACHED records the moment. Leaving REACHED forgets it.
    let reached_at: Option<DateTime<Utc>> = if state == "REACHED" {
        if previous == "REACHED" {
            row.try_get("reached_at")?
        } else {
            // Stored, so it is the database's clock.
            Some(
                sqlx::query_scalar::<_, DateTime<Utc>>("SELECT now()")
                    .fetch_one(&mut *tx)
                    .await?,
            )
        }
    } else {
        None
    };
    let updated = sqlx::query(&format!(
        "UPDATE penfii.milestones SET name=$2,description=$3,target_on=$4,state=$5,reached_at=$6,version=version+1,updated_at=now()
         WHERE id=$1 RETURNING {MILESTONE_COLUMNS}"
    ))
    .bind(milestone_id)
    .bind(&name)
    .bind(&description)
    .bind(target_on)
    .bind(&state)
    .bind(reached_at)
    .fetch_one(&mut *tx)
    .await?;
    let milestone = milestone_from(&updated)?;
    tx.commit().await?;
    Ok(milestone)
}

/// Replaces the set of Milestones a Ticket is linked to. New links need an open Milestone of
/// the same Project; links to a reached or canceled one can still be dropped.
pub async fn set_milestones(
    pool: &PgPool,
    actor_id: Uuid,
    ticket_id: Uuid,
    milestone_ids: Vec<Uuid>,
    idem: &str,
) -> Result<(Ticket, Option<ChangeNotice>), CoreError> {
    let wanted: Vec<Uuid> = {
        let mut seen = HashSet::new();
        milestone_ids
            .into_iter()
            .filter(|id| seen.insert(*id))
            .collect()
    };
    if wanted.len() > LIST_MAX {
        return Err(CoreError::validation("too many Milestones"));
    }
    let mut tx = pool.begin().await?;
    let ticket = load_live_ticket(&mut tx, ticket_id, true).await?;
    require_ticket_editor(&mut tx, ticket.project_id, ticket.channel_id, actor_id).await?;
    let body = json!({"ticketId":ticket_id,"milestoneIds":wanted});
    if let Some(done) = platform::replay::<Ticket>(
        &mut tx,
        ticket.workspace_id,
        actor_id,
        idem,
        "ticket.set_milestones",
        &body,
    )
    .await?
    {
        tx.commit().await?;
        return Ok((done, None));
    }
    let current_rows = sqlx::query(
        "SELECT m.id, m.name FROM penfii.ticket_milestones tm JOIN penfii.milestones m ON m.id=tm.milestone_id WHERE tm.ticket_id=$1",
    )
    .bind(ticket_id)
    .fetch_all(&mut *tx)
    .await?;
    let current: HashMap<Uuid, String> = current_rows
        .iter()
        .map(|r| -> Result<_, sqlx::Error> { Ok((r.try_get("id")?, r.try_get("name")?)) })
        .collect::<Result<HashMap<_, _>, sqlx::Error>>()?;
    let mut added = Vec::new();
    for id in wanted.iter().filter(|id| !current.contains_key(id)) {
        let row =
            sqlx::query("SELECT name,state FROM penfii.milestones WHERE id=$1 AND project_id=$2")
                .bind(id)
                .bind(ticket.project_id)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or_else(|| {
                    CoreError::validation("the Milestone is not part of this project")
                })?;
        if row.try_get::<String, _>("state")? != "OPEN" {
            return Err(CoreError::validation(
                "a Ticket can only be linked to an open Milestone",
            ));
        }
        added.push((*id, row.try_get::<String, _>("name")?));
    }
    let removed: Vec<(Uuid, String)> = current
        .iter()
        .filter(|(id, _)| !wanted.contains(id))
        .map(|(id, name)| (*id, name.clone()))
        .collect();
    let mut notice = None;
    if !added.is_empty() || !removed.is_empty() {
        for (id, _) in &added {
            sqlx::query("INSERT INTO penfii.ticket_milestones(ticket_id,milestone_id,linked_by) VALUES($1,$2,$3)")
                .bind(ticket_id)
                .bind(id)
                .bind(actor_id)
                .execute(&mut *tx)
                .await?;
        }
        for (id, _) in &removed {
            sqlx::query(
                "DELETE FROM penfii.ticket_milestones WHERE ticket_id=$1 AND milestone_id=$2",
            )
            .bind(ticket_id)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        }
        let names = |items: &[(Uuid, String)]| -> Vec<Value> {
            items
                .iter()
                .map(|(id, name)| json!({"id":id,"name":name}))
                .collect()
        };
        let (seq, _) = record_change(
            &mut tx,
            &ticket,
            actor_id,
            Uuid::new_v4(),
            "ticket.milestones_changed",
            json!({"ticketId":ticket_id,"added":names(&added),"removed":names(&removed)}),
        )
        .await?;
        notice = Some(ChangeNotice::Ticket {
            project_id: ticket.project_id,
            change_seq: seq,
            ticket_id,
            ticket_revision_no: ticket.revision_no,
            change_kind: "ticket.milestones_changed".into(),
        });
    }
    let mut value = load_live_ticket(&mut tx, ticket_id, false).await?;
    attach_derived(&mut tx, &mut value, actor_id).await?;
    attach_planned_end(&mut tx, &mut value).await?;
    platform::finish(&mut tx, ticket.workspace_id, actor_id, idem, 200, &value).await?;
    tx.commit().await?;
    Ok((value, notice))
}
