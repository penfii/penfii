//! Ticket comments.

use penfii_contracts::{TicketAttachment, TicketComment};
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool, Row};
use uuid::Uuid;

use super::changes::project_change;
use super::read_access;
use super::tickets::document_is_valid;
use crate::error::CoreError;
use crate::platform::ChangeNotice;
use crate::project_role::{can_manage, channel_allowed, require_project_role};

pub async fn comments(
    pool: &PgPool,
    actor_id: Uuid,
    ticket_id: Uuid,
) -> Result<Vec<TicketComment>, CoreError> {
    let mut tx = pool.begin().await?;
    let access = read_access(&mut tx, actor_id, ticket_id).await?;
    let moderator =
        can_moderate_comments(&mut tx, &access.role, access.channel_id, actor_id).await?;
    let rows=sqlx::query("SELECT c.id,c.ticket_id,c.author_id,u.display_name AS author_name,c.body,c.created_at,c.edited_at FROM penfii.ticket_comments c LEFT JOIN penfii.users u ON u.id=c.author_id WHERE c.ticket_id=$1 AND c.deleted_at IS NULL ORDER BY c.created_at,c.id").bind(ticket_id).fetch_all(&mut *tx).await?;
    let comment_ids: Vec<Uuid> = rows.iter().map(|r| r.get("id")).collect();
    let mut attachments =
        crate::ticket_files::for_comments(&mut tx, ticket_id, &comment_ids).await?;
    let values = rows
        .into_iter()
        .map(|r| {
            let attachments = attachments
                .remove(&r.get::<Uuid, _>("id"))
                .unwrap_or_default();
            comment_value(&r, attachments, actor_id, access.live, moderator)
        })
        .collect();
    tx.commit().await?;
    Ok(values)
}

/// Adds a comment (or Ticket file) change to the Project change feed and returns the notice that tells open boards and
/// Ticket panels to look again. A comment is not a Ticket revision, so the notice carries the
/// revision the Ticket already has.
pub(crate) async fn comment_change(
    conn: &mut PgConnection,
    project_id: Uuid,
    ticket_id: Uuid,
    kind: &str,
) -> Result<ChangeNotice, CoreError> {
    let revision_no: i64 = sqlx::query_scalar("SELECT revision_no FROM penfii.tickets WHERE id=$1")
        .bind(ticket_id)
        .fetch_one(&mut *conn)
        .await?;
    let change_seq = project_change(conn, project_id, ticket_id, revision_no, kind).await?;
    Ok(ChangeNotice::Ticket {
        project_id,
        change_seq,
        ticket_id,
        ticket_revision_no: revision_no,
        change_kind: kind.into(),
    })
}

pub async fn create_comment(
    pool: &PgPool,
    actor_id: Uuid,
    ticket_id: Uuid,
    body: Value,
    file_ids: &[Uuid],
) -> Result<(TicketComment, ChangeNotice), CoreError> {
    if body.is_null() {
        return Err(CoreError::validation("comment body is required"));
    }
    if !document_is_valid(&body, false) {
        return Err(CoreError::validation("comment body must be a JSON object"));
    }
    let mut tx = pool.begin().await?;
    let ticket=sqlx::query("SELECT workspace_id,project_id,channel_id FROM penfii.tickets WHERE id=$1 AND archived_at IS NULL AND trashed_at IS NULL AND purged_at IS NULL").bind(ticket_id).fetch_optional(&mut *tx).await?.ok_or(CoreError::NotFound)?;
    let workspace: Uuid = ticket.try_get("workspace_id")?;
    let project: Uuid = ticket.try_get("project_id")?;
    let channel: Option<Uuid> = ticket.try_get("channel_id")?;
    let _ = require_project_role(&mut tx, project, actor_id, false).await?;
    if !channel_allowed(&mut tx, channel, actor_id).await? {
        return Err(CoreError::NotFound);
    }
    let id = Uuid::new_v4();
    let value=sqlx::query("INSERT INTO penfii.ticket_comments(id,ticket_id,author_id,body) VALUES($1,$2,$3,$4) RETURNING id,ticket_id,author_id,(SELECT display_name FROM penfii.users WHERE id=$3) AS author_name,body,created_at,edited_at").bind(id).bind(ticket_id).bind(actor_id).bind(body).fetch_one(&mut *tx).await?;
    let mut comment = comment_value(&value, Vec::new(), actor_id, true, false);
    crate::ticket_files::attach_to_comment(&mut tx, actor_id, workspace, ticket_id, id, file_ids)
        .await?;
    comment.attachments = crate::ticket_files::for_comments(&mut tx, ticket_id, &[id])
        .await?
        .remove(&id)
        .unwrap_or_default();
    // A comment is a work event for Activity, but it is not a Ticket revision.
    sqlx::query(
        "INSERT INTO penfii.activity_entries(id,workspace_id,origin_project_id,origin_channel_id,ticket_id,actor_kind,actor_user_id,command_id,activity_kind,data,security_context)
         VALUES($1,$2,$3,$4,$5,'USER',$6,$7,'ticket.commented',$8,jsonb_build_object('projectId',$3::text,'channelId',$4::text))",
    ).bind(Uuid::new_v4()).bind(workspace).bind(project).bind(channel).bind(ticket_id).bind(actor_id).bind(Uuid::new_v4()).bind(json!({"commentId":id}))
     .execute(&mut *tx).await?;
    crate::events::emit(&mut tx, crate::events::NewEvent {
        workspace_id: workspace, target_kind: "TICKET", target_id: ticket_id, target_version: None,
        kind: "ticket.commented", actor_user_id: Some(actor_id), command_id: Uuid::new_v4(),
        facts: json!({"commentId": id, "projectId": project, "channelId": channel}),
        security_context: json!({"projectId": project.to_string(), "channelId": channel.map(|c| c.to_string())}),
    }).await?;
    let notice = comment_change(&mut tx, project, ticket_id, "ticket.commented").await?;
    tx.commit().await?;
    Ok((comment, notice))
}

/// Loads a live Ticket's comment for a change: the actor must be able to open the Ticket, and
/// archived or trashed Tickets take no writes (the same limit as creating a comment).
async fn writable_comment(
    tx: &mut PgConnection,
    actor_id: Uuid,
    ticket_id: Uuid,
    comment_id: Uuid,
) -> Result<(Uuid, Uuid, Uuid, Option<Uuid>, String), CoreError> {
    let ticket=sqlx::query("SELECT workspace_id,project_id,channel_id FROM penfii.tickets WHERE id=$1 AND archived_at IS NULL AND trashed_at IS NULL AND purged_at IS NULL").bind(ticket_id).fetch_optional(&mut *tx).await?.ok_or(CoreError::NotFound)?;
    let workspace: Uuid = ticket.try_get("workspace_id")?;
    let project: Uuid = ticket.try_get("project_id")?;
    let channel: Option<Uuid> = ticket.try_get("channel_id")?;
    let (_, role) = require_project_role(tx, project, actor_id, false).await?;
    if !channel_allowed(tx, channel, actor_id).await? {
        return Err(CoreError::NotFound);
    }
    let author: Uuid = sqlx::query_scalar("SELECT author_id FROM penfii.ticket_comments WHERE id=$1 AND ticket_id=$2 AND deleted_at IS NULL FOR UPDATE").bind(comment_id).bind(ticket_id).fetch_optional(&mut *tx).await?.ok_or(CoreError::NotFound)?;
    Ok((workspace, project, author, channel, role))
}

/// Only the author edits a comment. Editing is not a Ticket revision; the event refreshes search.
pub async fn edit_comment(
    pool: &PgPool,
    actor_id: Uuid,
    ticket_id: Uuid,
    comment_id: Uuid,
    body: Value,
) -> Result<(TicketComment, ChangeNotice), CoreError> {
    if body.is_null() {
        return Err(CoreError::validation("comment body is required"));
    }
    if !document_is_valid(&body, false) {
        return Err(CoreError::validation("comment body must be a JSON object"));
    }
    let mut tx = pool.begin().await?;
    let (workspace, project, author, channel, _) =
        writable_comment(&mut tx, actor_id, ticket_id, comment_id).await?;
    if author != actor_id {
        return Err(CoreError::forbidden("only the author can edit a comment"));
    }
    sqlx::query(
        "UPDATE penfii.ticket_comments SET body=$2, version=version+1, edited_at=now() WHERE id=$1",
    )
    .bind(comment_id)
    .bind(&body)
    .execute(&mut *tx)
    .await?;
    crate::events::emit(&mut tx, crate::events::NewEvent {
        workspace_id: workspace, target_kind: "TICKET", target_id: ticket_id, target_version: None,
        kind: "ticket.comment_edited", actor_user_id: Some(actor_id), command_id: Uuid::new_v4(),
        facts: json!({"commentId": comment_id, "projectId": project, "channelId": channel}),
        security_context: json!({"projectId": project.to_string(), "channelId": channel.map(|c| c.to_string())}),
    }).await?;
    let r=sqlx::query("SELECT c.id,c.ticket_id,c.author_id,u.display_name AS author_name,c.body,c.created_at,c.edited_at FROM penfii.ticket_comments c LEFT JOIN penfii.users u ON u.id=c.author_id WHERE c.id=$1").bind(comment_id).fetch_one(&mut *tx).await?;
    let attachments = crate::ticket_files::for_comments(&mut tx, ticket_id, &[comment_id])
        .await?
        .remove(&comment_id)
        .unwrap_or_default();
    let comment = comment_value(&r, attachments, actor_id, true, false);
    let notice = comment_change(&mut tx, project, ticket_id, "ticket.comment_edited").await?;
    tx.commit().await?;
    Ok((comment, notice))
}

/// A comment row with the requester's rights. `live` is false while the Ticket is archived or
/// trashed, which takes every write.
fn comment_value(
    r: &sqlx::postgres::PgRow,
    attachments: Vec<TicketAttachment>,
    actor_id: Uuid,
    live: bool,
    moderator: bool,
) -> TicketComment {
    let author: Uuid = r.get("author_id");
    TicketComment {
        id: r.get("id"),
        ticket_id: r.get("ticket_id"),
        author_id: author,
        author_name: r.get("author_name"),
        body: r.get("body"),
        created_at: r.get("created_at"),
        edited_at: r.get("edited_at"),
        attachments,
        can_edit: live && author == actor_id,
        can_delete: live && (author == actor_id || moderator),
    }
}

/// Who may remove other people's comments: a Project admin or a MANAGER of the Ticket's channel.
async fn can_moderate_comments(
    conn: &mut PgConnection,
    role: &str,
    channel: Option<Uuid>,
    actor_id: Uuid,
) -> Result<bool, CoreError> {
    if can_manage(role) {
        return Ok(true);
    }
    let Some(channel) = channel else {
        return Ok(false);
    };
    Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM penfii.channel_members WHERE channel_id=$1 AND user_id=$2 AND role='MANAGER')").bind(channel).bind(actor_id).fetch_one(&mut *conn).await?)
}

/// The author deletes their own comment; a Project admin or a MANAGER of the Ticket's channel may
/// remove anyone's. The attachments go with it.
pub async fn delete_comment(
    pool: &PgPool,
    store: Option<&crate::files::ObjectStore>,
    actor_id: Uuid,
    ticket_id: Uuid,
    comment_id: Uuid,
) -> Result<ChangeNotice, CoreError> {
    let mut tx = pool.begin().await?;
    let (workspace, project, author, channel, role) =
        writable_comment(&mut tx, actor_id, ticket_id, comment_id).await?;
    if author != actor_id && !can_moderate_comments(&mut tx, &role, channel, actor_id).await? {
        return Err(CoreError::forbidden(
            "only the author, a project admin or a channel manager can delete a comment",
        ));
    }
    sqlx::query("UPDATE penfii.ticket_comments SET body='{}'::jsonb, deleted_at=now() WHERE id=$1")
        .bind(comment_id)
        .execute(&mut *tx)
        .await?;
    let keys =
        crate::ticket_files::retire_comment(&mut tx, comment_id, workspace, actor_id).await?;
    crate::events::emit(&mut tx, crate::events::NewEvent {
        workspace_id: workspace, target_kind: "TICKET", target_id: ticket_id, target_version: None,
        kind: "ticket.comment_deleted", actor_user_id: Some(actor_id), command_id: Uuid::new_v4(),
        facts: json!({"commentId": comment_id, "projectId": project, "channelId": channel}),
        security_context: json!({"projectId": project.to_string(), "channelId": channel.map(|c| c.to_string())}),
    }).await?;
    crate::files::delete_objects(store, &keys).await?;
    let notice = comment_change(&mut tx, project, ticket_id, "ticket.comment_deleted").await?;
    tx.commit().await?;
    Ok(notice)
}
