//! Ticket dependency edges, their waivers and the review they can need.

use penfii_contracts::{DependencyEdge, Ticket, TicketDependencies, TicketRef};
use serde_json::json;
use sqlx::{PgConnection, PgPool, Row};
use std::collections::HashMap;
use uuid::Uuid;

use super::derived::visible;
use super::is_unique;
use super::order::lock_project;
use crate::error::CoreError;
use crate::platform::{self, ChangeNotice};
use crate::project_role::{channel_allowed, require_project_role};
use crate::work::{load_live_ticket, load_readable_ticket, record_change};

fn edge_from(
    row: &sqlx::postgres::PgRow,
    satisfied: bool,
) -> Result<(DependencyEdge, Option<Uuid>), CoreError> {
    let channel: Option<Uuid> = row.try_get("channel_id")?;
    let category: String = row.try_get("category")?;
    let kind: String = row.try_get("kind")?;
    let is_blocker: bool = row.try_get("is_blocker")?;
    let waived: bool = row.try_get("waived")?;
    Ok((
        DependencyEdge {
            id: row.try_get("id")?,
            ticket: TicketRef {
                id: row.try_get("other_id")?,
                key: row.try_get("public_key")?,
                title: row.try_get("title")?,
                hierarchy_kind: row.try_get("hierarchy_kind")?,
            },
            state_name: row.try_get("state_name")?,
            predecessor_canceled: kind == "BLOCKS" && is_blocker && category == "CANCELED",
            state_category: category,
            satisfied,
            created_at: row.try_get("created_at")?,
            kind,
            waived,
            waive_reason: row.try_get("waive_reason")?,
            needs_review: false,
        },
        channel,
    ))
}

/// A forced dependency whose predecessor sits in a private channel the successor is not part of
/// no longer fits the visibility rules (the channel may have become private after the link).
async fn dependency_needs_review(
    conn: &mut PgConnection,
    predecessor_channel: Option<Uuid>,
    successor_channel: Option<Uuid>,
) -> Result<bool, CoreError> {
    let Some(pred) = predecessor_channel else {
        return Ok(false);
    };
    if Some(pred) == successor_channel {
        return Ok(false);
    }
    Ok(
        sqlx::query_scalar("SELECT visibility='PRIVATE' FROM penfii.channels WHERE id=$1")
            .bind(pred)
            .fetch_one(&mut *conn)
            .await?,
    )
}

async fn dependencies_of(
    conn: &mut PgConnection,
    ticket: &Ticket,
    actor_id: Uuid,
    role: &str,
) -> Result<TicketDependencies, CoreError> {
    let can_edit = crate::project_role::can_collaborate(role);
    let own_category: String =
        sqlx::query_scalar("SELECT category FROM penfii.workflow_statuses WHERE id=$1")
            .bind(ticket.status_id)
            .fetch_one(&mut *conn)
            .await?;
    let rows = sqlx::query(
        "SELECT d.id, d.created_at, d.kind, (d.waived_at IS NOT NULL) AS waived, d.waive_reason,
                (d.successor_id=$1) AS is_blocker,
                o.id AS other_id, o.public_key, o.title, o.hierarchy_kind, o.channel_id, o.project_id AS other_project,
                s.name AS state_name, s.category
         FROM penfii.ticket_dependencies d
         JOIN penfii.tickets o ON o.id=CASE WHEN d.successor_id=$1 THEN d.predecessor_id ELSE d.successor_id END
         JOIN penfii.workflow_statuses s ON s.id=o.status_id
         WHERE (d.successor_id=$1 OR d.predecessor_id=$1)
           AND o.trashed_at IS NULL AND o.purged_at IS NULL
         ORDER BY d.created_at, d.id",
    )
    .bind(ticket.id)
    .fetch_all(&mut *conn)
    .await?;
    let mut access = HashMap::new();
    let mut blocked_by = Vec::new();
    let mut blocks = Vec::new();
    let mut related = Vec::new();
    for row in rows {
        let is_blocker: bool = row.try_get("is_blocker")?;
        let category: String = row.try_get("category")?;
        let kind: String = row.try_get("kind")?;
        let waived: bool = row.try_get("waived")?;
        let satisfied = kind == "RELATED"
            || waived
            || if is_blocker {
                category == "DONE"
            } else {
                own_category == "DONE"
            };
        let (mut edge, channel) = edge_from(&row, satisfied)?;
        if !visible(conn, &mut access, channel, actor_id).await? {
            continue;
        }
        let other_project: Uuid = row.try_get("other_project")?;
        if other_project != ticket.project_id
            && crate::project_role::project_role(conn, other_project, actor_id)
                .await?
                .1
                .is_none()
        {
            continue;
        }
        if kind == "BLOCKS" {
            let (pred_channel, succ_channel) = if is_blocker {
                (channel, ticket.channel_id)
            } else {
                (ticket.channel_id, channel)
            };
            edge.needs_review = dependency_needs_review(conn, pred_channel, succ_channel).await?;
        }
        if kind == "RELATED" {
            related.push(edge);
        } else if is_blocker {
            blocked_by.push(edge);
        } else {
            blocks.push(edge);
        }
    }
    Ok(TicketDependencies {
        ticket_id: ticket.id,
        blocked_by,
        blocks,
        related,
        can_edit,
        can_waive: crate::project_role::can_manage_or_designated(
            conn,
            ticket.project_id,
            actor_id,
            role,
            crate::project_role::Designation::WaiveDependencies,
        )
        .await?,
    })
}

pub async fn ticket_dependencies(
    pool: &PgPool,
    actor_id: Uuid,
    ticket_id: Uuid,
) -> Result<TicketDependencies, CoreError> {
    let mut tx = pool.begin().await?;
    let ticket = load_readable_ticket(&mut tx, ticket_id).await?;
    let (_, role) = require_project_role(&mut tx, ticket.project_id, actor_id, false).await?;
    if !channel_allowed(&mut tx, ticket.channel_id, actor_id).await? {
        return Err(CoreError::NotFound);
    }
    let value = dependencies_of(&mut tx, &ticket, actor_id, &role).await?;
    tx.commit().await?;
    Ok(value)
}

/// Is `descendant` below `ancestor` in the Ticket hierarchy?
async fn is_below(
    conn: &mut PgConnection,
    ancestor: Uuid,
    descendant: Uuid,
) -> Result<bool, CoreError> {
    Ok(sqlx::query_scalar(
        "WITH RECURSIVE up(id) AS (
            SELECT parent_id FROM penfii.tickets WHERE id=$2
            UNION
            SELECT t.parent_id FROM penfii.tickets t JOIN up ON t.id=up.id
         ) SELECT EXISTS(SELECT 1 FROM up WHERE id=$1)",
    )
    .bind(ancestor)
    .bind(descendant)
    .fetch_one(&mut *conn)
    .await?)
}

/// The checks that need the Project lock `add_dependency` already holds: one relation per pair
/// whatever its kind or direction, and no cycle among `BLOCKS` edges. Who may add the relation was
/// decided by the command before this runs.
async fn ensure_relation_is_new(
    tx: &mut PgConnection,
    related: bool,
    predecessor_id: Uuid,
    successor_id: Uuid,
) -> Result<(), CoreError> {
    // One relation per pair, whatever its kind or direction.
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM penfii.ticket_dependencies
         WHERE (predecessor_id=$1 AND successor_id=$2) OR (predecessor_id=$2 AND successor_id=$1))",
    )
    .bind(predecessor_id)
    .bind(successor_id)
    .fetch_one(&mut *tx)
    .await?;
    if exists && related {
        return Err(CoreError::conflict(
            "dependency_exists",
            "these Tickets are already related",
        ));
    }
    if !related {
        let same_direction: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM penfii.ticket_dependencies WHERE predecessor_id=$1 AND successor_id=$2)",
        )
        .bind(predecessor_id)
        .bind(successor_id)
        .fetch_one(&mut *tx)
        .await?;
        if same_direction {
            return Err(CoreError::conflict(
                "dependency_exists",
                "this dependency already exists",
            ));
        }
        let reversed_related: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM penfii.ticket_dependencies WHERE predecessor_id=$2 AND successor_id=$1 AND kind='RELATED')",
        )
        .bind(predecessor_id)
        .bind(successor_id)
        .fetch_one(&mut *tx)
        .await?;
        if reversed_related {
            return Err(CoreError::conflict(
                "dependency_exists",
                "these Tickets are already related",
            ));
        }
        // Adding predecessor -> successor closes a loop when the predecessor is already downstream.
        let cycle: bool = sqlx::query_scalar(
            "WITH RECURSIVE downstream(id) AS (
                SELECT $1::uuid
                UNION
                SELECT d.successor_id FROM penfii.ticket_dependencies d JOIN downstream x ON d.predecessor_id=x.id
                WHERE d.kind='BLOCKS'
             ) SELECT EXISTS(SELECT 1 FROM downstream WHERE id=$2)",
        )
        .bind(successor_id)
        .bind(predecessor_id)
        .fetch_one(&mut *tx)
        .await?;
        if cycle {
            return Err(CoreError::conflict(
                "dependency_cycle",
                "this dependency would make Tickets wait for each other",
            ));
        }
    }
    Ok(())
}

/// Adds a relation between two Tickets. `BLOCKS` makes `successor` wait for `predecessor` and both
/// stay inside one Project with an acyclic graph. `RELATED` is a plain link that can cross Projects
/// of the Workspace; the pair has no direction, and one side must be editable by the actor.
pub async fn add_dependency(
    pool: &PgPool,
    actor_id: Uuid,
    successor_id: Uuid,
    predecessor_id: Uuid,
    kind: &str,
    idem: &str,
) -> Result<(TicketDependencies, Option<ChangeNotice>), CoreError> {
    if !matches!(kind, "BLOCKS" | "RELATED") {
        return Err(CoreError::validation("kind must be BLOCKS or RELATED"));
    }
    let related = kind == "RELATED";
    if successor_id == predecessor_id {
        return Err(CoreError::validation(if related {
            "a Ticket cannot be related to itself"
        } else {
            "a Ticket cannot depend on itself"
        }));
    }
    let mut tx = pool.begin().await?;
    let successor = load_live_ticket(&mut tx, successor_id, true).await?;
    let (_, successor_role) =
        require_project_role(&mut tx, successor.project_id, actor_id, false).await?;
    if !channel_allowed(&mut tx, successor.channel_id, actor_id).await? {
        return Err(CoreError::NotFound);
    }
    let successor_editor = crate::project_role::can_collaborate(&successor_role);
    if !related && !successor_editor {
        return Err(CoreError::forbidden("project editor required"));
    }
    let body = json!({"successorId":successor_id,"predecessorId":predecessor_id,"kind":kind});
    if let Some(done) = platform::replay::<TicketDependencies>(
        &mut tx,
        successor.workspace_id,
        actor_id,
        idem,
        "ticket.dependency_add",
        &body,
    )
    .await?
    {
        tx.commit().await?;
        return Ok((done, None));
    }
    // A hidden or missing predecessor looks the same: not found.
    let predecessor = load_live_ticket(&mut tx, predecessor_id, false).await?;
    if predecessor.workspace_id != successor.workspace_id
        || !channel_allowed(&mut tx, predecessor.channel_id, actor_id).await?
    {
        return Err(CoreError::NotFound);
    }
    let (_, predecessor_role) =
        crate::project_role::project_role(&mut tx, predecessor.project_id, actor_id).await?;
    let Some(predecessor_role) = predecessor_role else {
        return Err(CoreError::NotFound);
    };
    if related {
        let predecessor_editor = crate::project_role::can_collaborate(&predecessor_role);
        if !successor_editor && !predecessor_editor {
            return Err(CoreError::forbidden(
                "edit permission on one of the two Tickets is required",
            ));
        }
    } else if predecessor.project_id != successor.project_id {
        return Err(CoreError::validation(
            "a dependency must connect Tickets of the same Project",
        ));
    }
    if !related
        && let Some(pred_channel) = predecessor.channel_id
        && Some(pred_channel) != successor.channel_id
    {
        let private: bool =
            sqlx::query_scalar("SELECT visibility='PRIVATE' FROM penfii.channels WHERE id=$1")
                .bind(pred_channel)
                .fetch_one(&mut *tx)
                .await?;
        if private {
            return Err(CoreError::validation(
                "a Ticket in a private channel can only block Tickets of the same channel",
            ));
        }
    }
    lock_project(&mut tx, successor.project_id).await?;
    if !related
        && (is_below(&mut tx, predecessor_id, successor_id).await?
            || is_below(&mut tx, successor_id, predecessor_id).await?)
    {
        return Err(CoreError::validation(
            "a Ticket cannot depend on its own parent or child",
        ));
    }
    ensure_relation_is_new(&mut tx, related, predecessor_id, successor_id).await?;
    let inserted = sqlx::query(
        "INSERT INTO penfii.ticket_dependencies(id,workspace_id,project_id,predecessor_id,successor_id,created_by,kind)
         VALUES($1,$2,$3,$4,$5,$6,$7)",
    )
    .bind(Uuid::new_v4())
    .bind(successor.workspace_id)
    .bind(successor.project_id)
    .bind(predecessor_id)
    .bind(successor_id)
    .bind(actor_id)
    .bind(kind)
    .execute(&mut *tx)
    .await;
    match inserted {
        Ok(_) => {}
        Err(error) if is_unique(&error) => {
            return Err(CoreError::conflict(
                "dependency_exists",
                "these Tickets are already related",
            ));
        }
        Err(error) => return Err(error.into()),
    }
    let change_kind = if related {
        "ticket.related_added"
    } else {
        "ticket.dependency_added"
    };
    let (seq, _) = record_change(
        &mut tx,
        &successor,
        actor_id,
        Uuid::new_v4(),
        change_kind,
        json!({"ticketId":successor_id,"predecessorId":predecessor_id,"predecessorKey":predecessor.key,"kind":kind}),
    )
    .await?;
    let value = dependencies_of(&mut tx, &successor, actor_id, &successor_role).await?;
    platform::finish(&mut tx, successor.workspace_id, actor_id, idem, 201, &value).await?;
    tx.commit().await?;
    Ok((
        value,
        Some(ChangeNotice::Ticket {
            project_id: successor.project_id,
            change_seq: seq,
            ticket_id: successor_id,
            ticket_revision_no: successor.revision_no,
            change_kind: change_kind.into(),
        }),
    ))
}

/// Removes a relation.
///
/// - A RELATED link: edit permission on either Ticket.
/// - A dependency whose predecessor is DONE or CANCELED: edit permission on the successor. A
///   canceled predecessor can never be completed, so keeping the relation guards nothing; the
///   removal is logged.
/// - A dependency that still blocks (predecessor open, or waived): a Project admin.
pub async fn remove_dependency(
    pool: &PgPool,
    actor_id: Uuid,
    dependency_id: Uuid,
) -> Result<Option<ChangeNotice>, CoreError> {
    let mut tx = pool.begin().await?;
    let row = sqlx::query(
        "SELECT predecessor_id, successor_id, kind FROM penfii.ticket_dependencies WHERE id=$1 FOR UPDATE",
    )
    .bind(dependency_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(CoreError::NotFound)?;
    let predecessor_id: Uuid = row.try_get("predecessor_id")?;
    let successor_id: Uuid = row.try_get("successor_id")?;
    let kind: String = row.try_get("kind")?;
    let successor = load_live_ticket(&mut tx, successor_id, true).await?;
    let (_, role) = require_project_role(&mut tx, successor.project_id, actor_id, false).await?;
    if !channel_allowed(&mut tx, successor.channel_id, actor_id).await? {
        return Err(CoreError::NotFound);
    }
    let predecessor = load_live_ticket(&mut tx, predecessor_id, false).await.ok();
    let editor = crate::project_role::can_collaborate(&role);
    if kind == "RELATED" {
        let mut allowed = editor;
        if !allowed && let Some(p) = &predecessor {
            let (_, other) =
                crate::project_role::project_role(&mut tx, p.project_id, actor_id).await?;
            allowed = other.is_some_and(|r| crate::project_role::can_collaborate(&r))
                && channel_allowed(&mut tx, p.channel_id, actor_id).await?;
        }
        if !allowed {
            return Err(CoreError::forbidden(
                "edit permission on one of the two Tickets is required",
            ));
        }
    } else {
        if !editor {
            return Err(CoreError::forbidden("project editor required"));
        }
        let category: Option<String> = match &predecessor {
            Some(p) => Some(
                sqlx::query_scalar("SELECT category FROM penfii.workflow_statuses WHERE id=$1")
                    .bind(p.status_id)
                    .fetch_one(&mut *tx)
                    .await?,
            ),
            None => None,
        };
        let guarded = category
            .as_deref()
            .is_some_and(|c| c != "DONE" && c != "CANCELED");
        if guarded
            && !crate::project_role::can_manage_or_designated(
                &mut tx,
                successor.project_id,
                actor_id,
                &role,
                crate::project_role::Designation::WaiveDependencies,
            )
            .await?
        {
            return Err(CoreError::forbidden(
                "a dependency that still blocks can only be removed by a project admin or a designated editor",
            ));
        }
    }
    lock_project(&mut tx, successor.project_id).await?;
    let category_after: Option<String> = match &predecessor {
        Some(p) => Some(
            sqlx::query_scalar("SELECT category FROM penfii.workflow_statuses WHERE id=$1")
                .bind(p.status_id)
                .fetch_one(&mut *tx)
                .await?,
        ),
        None => None,
    };
    sqlx::query("DELETE FROM penfii.ticket_dependencies WHERE id=$1")
        .bind(dependency_id)
        .execute(&mut *tx)
        .await?;
    let key = predecessor.as_ref().map(|p| p.key.clone());
    let change_kind = if kind == "RELATED" {
        "ticket.related_removed"
    } else {
        "ticket.dependency_removed"
    };
    let (seq, _) = record_change(
        &mut tx,
        &successor,
        actor_id,
        Uuid::new_v4(),
        change_kind,
        json!({"ticketId":successor_id,"predecessorId":predecessor_id,"predecessorKey":key,"kind":kind,"predecessorCategory":category_after}),
    )
    .await?;
    tx.commit().await?;
    Ok(Some(ChangeNotice::Ticket {
        project_id: successor.project_id,
        change_seq: seq,
        ticket_id: successor_id,
        ticket_revision_no: successor.revision_no,
        change_kind: change_kind.into(),
    }))
}

/// Waives (`waive = true`) or restores a forced dependency. The relation stays in place; while
/// waived it no longer blocks. Both acts need a Project admin of the successor and a reason, and
/// both are kept in the waiver history and in the Ticket's activity.
pub async fn set_waiver(
    pool: &PgPool,
    actor_id: Uuid,
    dependency_id: Uuid,
    waive: bool,
    reason: &str,
    idem: &str,
) -> Result<(TicketDependencies, Option<ChangeNotice>), CoreError> {
    let reason = reason.trim();
    if reason.is_empty() || reason.chars().count() > 300 {
        return Err(CoreError::validation(
            "a reason of 1..300 characters is required",
        ));
    }
    let mut tx = pool.begin().await?;
    let row = sqlx::query(
        "SELECT predecessor_id, successor_id, kind, waived_at FROM penfii.ticket_dependencies WHERE id=$1 FOR UPDATE",
    )
    .bind(dependency_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(CoreError::NotFound)?;
    let predecessor_id: Uuid = row.try_get("predecessor_id")?;
    let successor_id: Uuid = row.try_get("successor_id")?;
    let kind: String = row.try_get("kind")?;
    let waived: Option<chrono::DateTime<chrono::Utc>> = row.try_get("waived_at")?;
    let successor = load_live_ticket(&mut tx, successor_id, true).await?;
    let (_, role) = require_project_role(&mut tx, successor.project_id, actor_id, false).await?;
    if !channel_allowed(&mut tx, successor.channel_id, actor_id).await? {
        return Err(CoreError::NotFound);
    }
    if !crate::project_role::can_manage_or_designated(
        &mut tx,
        successor.project_id,
        actor_id,
        &role,
        crate::project_role::Designation::WaiveDependencies,
    )
    .await?
    {
        return Err(CoreError::forbidden(
            "only a project admin or a designated editor can waive or restore a dependency",
        ));
    }
    let body = json!({"dependencyId":dependency_id,"waive":waive,"reason":reason});
    if let Some(done) = platform::replay::<TicketDependencies>(
        &mut tx,
        successor.workspace_id,
        actor_id,
        idem,
        "ticket.dependency_waiver",
        &body,
    )
    .await?
    {
        tx.commit().await?;
        return Ok((done, None));
    }
    if kind != "BLOCKS" {
        return Err(CoreError::validation("only a dependency can be waived"));
    }
    if waive == waived.is_some() {
        return Err(CoreError::conflict(
            "invalid_waiver_state",
            if waive {
                "this dependency is already waived"
            } else {
                "this dependency is not waived"
            },
        ));
    }
    let predecessor = load_live_ticket(&mut tx, predecessor_id, false).await?;
    if waive {
        let category: String =
            sqlx::query_scalar("SELECT category FROM penfii.workflow_statuses WHERE id=$1")
                .bind(predecessor.status_id)
                .fetch_one(&mut *tx)
                .await?;
        if category == "DONE" {
            return Err(CoreError::conflict(
                "dependency_satisfied",
                "this dependency is already satisfied",
            ));
        }
    }
    lock_project(&mut tx, successor.project_id).await?;
    sqlx::query(
        "UPDATE penfii.ticket_dependencies
         SET waived_at=CASE WHEN $2 THEN now() END, waived_by=CASE WHEN $2 THEN $3::uuid END,
             waive_reason=CASE WHEN $2 THEN $4 END
         WHERE id=$1",
    )
    .bind(dependency_id)
    .bind(waive)
    .bind(actor_id)
    .bind(reason)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO penfii.dependency_waiver_history(id,workspace_id,project_id,dependency_id,predecessor_id,successor_id,action,reason,actor_id)
         VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)",
    )
    .bind(Uuid::new_v4())
    .bind(successor.workspace_id)
    .bind(successor.project_id)
    .bind(dependency_id)
    .bind(predecessor_id)
    .bind(successor_id)
    .bind(if waive { "WAIVED" } else { "RESTORED" })
    .bind(reason)
    .bind(actor_id)
    .execute(&mut *tx)
    .await?;
    let change_kind = if waive {
        "ticket.dependency_waived"
    } else {
        "ticket.dependency_restored"
    };
    let (seq, _) = record_change(
        &mut tx,
        &successor,
        actor_id,
        Uuid::new_v4(),
        change_kind,
        json!({"ticketId":successor_id,"predecessorId":predecessor_id,"predecessorKey":predecessor.key,"reason":reason}),
    )
    .await?;
    let value = dependencies_of(&mut tx, &successor, actor_id, &role).await?;
    platform::finish(&mut tx, successor.workspace_id, actor_id, idem, 200, &value).await?;
    tx.commit().await?;
    Ok((
        value,
        Some(ChangeNotice::Ticket {
            project_id: successor.project_id,
            change_seq: seq,
            ticket_id: successor_id,
            ticket_revision_no: successor.revision_no,
            change_kind: change_kind.into(),
        }),
    ))
}
