//! Ticket snapshots, and the change records and notices a Ticket write leaves behind.

use penfii_contracts::Ticket;
use serde_json::{Value, json};
use sqlx::{PgConnection, Row};
use std::collections::HashMap;
use uuid::Uuid;

use super::activity::is_link_activity;
use crate::error::CoreError;

/// Snapshot of the fields Revision, History and restore use. It keeps the display values
/// (status, type, assignee, labels, parent key) that were true when the revision was written.
pub(super) async fn snapshot_value(
    conn: &mut PgConnection,
    ticket: &Ticket,
) -> Result<Value, CoreError> {
    let status = sqlx::query("SELECT name,category FROM penfii.workflow_statuses WHERE id=$1")
        .bind(ticket.status_id)
        .fetch_optional(&mut *conn)
        .await?;
    let (status_name, status_category): (Option<String>, Option<String>) = match status {
        Some(row) => (Some(row.try_get("name")?), Some(row.try_get("category")?)),
        None => (None, None),
    };
    Ok(json!({
        "key": ticket.key,
        "title": ticket.title,
        "description": ticket.description,
        "hierarchyKind": ticket.hierarchy_kind,
        "projectId": ticket.project_id,
        "channelId": ticket.channel_id,
        "parentId": ticket.parent_id,
        "parentKey": ticket.parent.as_ref().map(|p| p.key.clone()),
        "statusId": ticket.status_id,
        "statusName": status_name,
        "statusCategory": status_category,
        "assigneeId": ticket.assignee_id,
        "assigneeName": ticket.assignee_name,
        "priority": ticket.priority,
        "dueOn": ticket.due_on,
        "typeId": ticket.ticket_type.as_ref().map(|t| t.id),
        "typeName": ticket.ticket_type.as_ref().map(|t| t.name.clone()),
        "labels": ticket.labels,
        "estimateValue": ticket.estimate.as_ref().map(|e| e.value.clone()),
        "estimateUnit": ticket.estimate.as_ref().map(|e| e.unit.clone()),
        "schedule": ticket.schedule.as_ref().map(|s| json!({
            "plannedStartAt": s.planned_start_at,
            "durationMinutes": s.duration_minutes,
            "isScheduleMilestone": s.is_schedule_milestone,
        })),
        "revisionNo": ticket.revision_no,
    }))
}

pub(super) async fn save_snapshot(
    conn: &mut PgConnection,
    ticket: &Ticket,
    actor_id: Uuid,
    command_id: Uuid,
    restore: Option<(i64, Value)>,
) -> Result<(), CoreError> {
    let snapshot = snapshot_value(conn, ticket).await?;
    let (restore_source, restore_metadata) = match restore {
        Some((source, metadata)) => (Some(source), Some(metadata)),
        None => (None, None),
    };
    sqlx::query(
        "INSERT INTO penfii.ticket_revisions(ticket_id,revision_no,actor_kind,actor_user_id,initiated_by_user_id,command_id,snapshot_schema_version,snapshot,security_context,restore_source_revision_no,restore_metadata)
         VALUES($1,$2,'USER',$3,$3,$4,1,$5,jsonb_build_object('projectId',$6::text,'channelId',$7::text),$8,$9)",
    ).bind(ticket.id).bind(ticket.revision_no).bind(actor_id).bind(command_id).bind(snapshot)
     .bind(ticket.project_id).bind(ticket.channel_id).bind(restore_source).bind(restore_metadata).execute(&mut *conn).await?;
    Ok(())
}

pub(super) async fn status_names(
    conn: &mut PgConnection,
    project_id: Uuid,
) -> Result<HashMap<Uuid, String>, CoreError> {
    let rows = sqlx::query("SELECT id,name FROM penfii.workflow_statuses WHERE project_id=$1")
        .bind(project_id)
        .fetch_all(&mut *conn)
        .await?;
    rows.into_iter()
        .map(|row| Ok((row.try_get("id")?, row.try_get("name")?)))
        .collect()
}

/// Appends one entry to the Project change feed. The Project row lock serializes writers, so
/// sequence numbers never repeat.
pub(crate) async fn project_change(
    conn: &mut PgConnection,
    project_id: Uuid,
    ticket_id: Uuid,
    revision_no: i64,
    kind: &str,
) -> Result<i64, CoreError> {
    sqlx::query("SELECT id FROM penfii.projects WHERE id=$1 FOR UPDATE")
        .bind(project_id)
        .fetch_one(&mut *conn)
        .await?;
    let seq: i64 = sqlx::query_scalar(
        "INSERT INTO penfii.project_ticket_changes(project_id,change_seq,ticket_id,ticket_revision_no,change_kind)
         SELECT $1, coalesce(max(change_seq),0)+1, $2, $3, $4 FROM penfii.project_ticket_changes WHERE project_id=$1
         RETURNING change_seq",
    ).bind(project_id).bind(ticket_id).bind(revision_no).bind(kind).fetch_one(&mut *conn).await?;
    Ok(seq)
}

/// The fields a View or Search can filter on, taken from a revision snapshot, plus the planning
/// values (estimate, schedule) that events carry.
fn filterable(snapshot: &Value) -> Value {
    let labels: Vec<Value> = snapshot
        .get("labels")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(|i| i.get("id").cloned()).collect())
        .unwrap_or_default();
    json!({
        "projectId": snapshot.get("projectId"),
        "channelId": snapshot.get("channelId"),
        "statusId": snapshot.get("statusId"),
        "statusCategory": snapshot.get("statusCategory"),
        "typeId": snapshot.get("typeId"),
        "priority": snapshot.get("priority"),
        "assigneeId": snapshot.get("assigneeId"),
        "labelIds": labels,
        "dueOn": snapshot.get("dueOn"),
        "parentId": snapshot.get("parentId"),
        "title": snapshot.get("title"),
        "estimateValue": snapshot.get("estimateValue"),
        "estimateUnit": snapshot.get("estimateUnit"),
        "schedule": snapshot.get("schedule"),
    })
}

/// Adds `before` and `after` of every filterable field to an event's facts. `after` is the
/// Ticket now; `before` is the previous revision when this event produced a new revision, and
/// the same as `after` for events that only link or plan (they change no field).
async fn with_before_after(
    conn: &mut PgConnection,
    ticket: &Ticket,
    kind: &str,
    facts: Value,
) -> Result<Value, CoreError> {
    let after = filterable(&snapshot_value(conn, ticket).await?);
    let before = if ticket.revision_no > 1 && !is_link_activity(kind) {
        let previous: Option<Value> = sqlx::query_scalar(
            "SELECT snapshot FROM penfii.ticket_revisions WHERE ticket_id=$1 AND revision_no=$2",
        )
        .bind(ticket.id)
        .bind(ticket.revision_no - 1)
        .fetch_optional(&mut *conn)
        .await?;
        match previous {
            Some(snapshot) => {
                let mut before = filterable(&snapshot);
                if before["statusCategory"].is_null()
                    && let Some(id) = snapshot
                        .get("statusId")
                        .and_then(Value::as_str)
                        .and_then(|t| Uuid::parse_str(t).ok())
                {
                    let category: Option<String> = sqlx::query_scalar(
                        "SELECT category FROM penfii.workflow_statuses WHERE id=$1",
                    )
                    .bind(id)
                    .fetch_optional(&mut *conn)
                    .await?;
                    before["statusCategory"] = json!(category);
                }
                before
            }
            None => after.clone(),
        }
    } else {
        after.clone()
    };
    let mut facts = facts;
    if let Value::Object(map) = &mut facts {
        map.insert("before".into(), before);
        map.insert("after".into(), after);
    }
    Ok(facts)
}

pub(crate) async fn record_change(
    conn: &mut PgConnection,
    ticket: &Ticket,
    actor_id: Uuid,
    command_id: Uuid,
    kind: &str,
    facts: Value,
) -> Result<(i64, Uuid), CoreError> {
    let seq = project_change(conn, ticket.project_id, ticket.id, ticket.revision_no, kind).await?;
    sqlx::query(
        "INSERT INTO penfii.activity_entries(id,workspace_id,origin_project_id,origin_channel_id,ticket_id,ticket_revision_no,actor_kind,actor_user_id,command_id,activity_kind,data,security_context)
         VALUES($1,$2,$3,$4,$5,$6,'USER',$7,$8,$9,$10,jsonb_build_object('projectId',$3::text,'channelId',$4::text))",
    ).bind(Uuid::new_v4()).bind(ticket.workspace_id).bind(ticket.project_id).bind(ticket.channel_id)
     .bind(ticket.id).bind(ticket.revision_no).bind(actor_id).bind(command_id).bind(kind).bind(facts.clone())
     .execute(&mut *conn).await?;
    let facts = with_before_after(conn, ticket, kind, facts).await?;
    let event_id = crate::events::emit(
        conn,
        crate::events::NewEvent {
            workspace_id: ticket.workspace_id,
            target_kind: "TICKET",
            target_id: ticket.id,
            target_version: Some(ticket.revision_no),
            kind,
            actor_user_id: Some(actor_id),
            command_id,
            facts,
            security_context: json!({
                "projectId": ticket.project_id.to_string(),
                "channelId": ticket.channel_id.map(|c| c.to_string()),
            }),
        },
    )
    .await?;
    Ok((seq, event_id))
}

/// Records the category a State has from now on. Reports read the past through this history and
/// the Ticket revisions, never through the current Workflow.
pub(crate) async fn record_status_category(
    conn: &mut PgConnection,
    status_id: Uuid,
    category: &str,
) -> Result<(), CoreError> {
    sqlx::query("INSERT INTO penfii.workflow_status_history(status_id,category) VALUES($1,$2)")
        .bind(status_id)
        .bind(category)
        .execute(&mut *conn)
        .await?;
    Ok(())
}
