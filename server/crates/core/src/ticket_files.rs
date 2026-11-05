//! Files attached to a Ticket: to its body or to one of its comments.
//!
//! Binding goes through `files::claim_unbound`, the same rule messages use, and reading follows the
//! Ticket itself: whoever can open the live Ticket can open its files, and nobody can once it is in
//! the trash, archived or purged. There is no separate sharing scope and no Ticket-specific limit;
//! the single-file limit of the shared upload applies.

use std::collections::HashMap;

use penfii_contracts::{MessageAttachment, TicketAttachment};
use sqlx::{PgConnection, PgPool, Row};
use uuid::Uuid;

use crate::error::CoreError;
use crate::events;
use crate::files::{self, ObjectStore};
use crate::platform::ChangeNotice;
use crate::project_role;
use crate::work;

const FROM: &str = "FROM penfii.ticket_attachments a JOIN penfii.files f ON f.id = a.file_id";
const COLUMNS: &str = "a.id, a.ticket_id, a.comment_id, a.file_id, f.filename, f.byte_size,
        f.detected_mime, f.state, a.attached_by, a.attached_at";

fn attachment(row: &sqlx::postgres::PgRow) -> Result<TicketAttachment, sqlx::Error> {
    Ok(TicketAttachment {
        id: row.try_get("id")?,
        ticket_id: row.try_get("ticket_id")?,
        comment_id: row.try_get("comment_id")?,
        file_id: row.try_get("file_id")?,
        filename: row.try_get("filename")?,
        byte_size: row.try_get("byte_size")?,
        mime: row.try_get("detected_mime")?,
        state: row.try_get("state")?,
        attached_by: row.try_get("attached_by")?,
        attached_at: row.try_get("attached_at")?,
    })
}

struct LiveTicket {
    workspace_id: Uuid,
    project_id: Uuid,
}

/// The Ticket when it is live (not archived, trashed or purged) and the person may open it.
async fn readable_ticket(
    conn: &mut PgConnection,
    actor: Uuid,
    ticket_id: Uuid,
) -> Result<LiveTicket, CoreError> {
    let row = sqlx::query(
        "SELECT workspace_id, project_id, channel_id FROM penfii.tickets
         WHERE id = $1 AND archived_at IS NULL AND trashed_at IS NULL AND purged_at IS NULL
         FOR SHARE",
    )
    .bind(ticket_id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(CoreError::NotFound)?;
    let project_id: Uuid = row.try_get("project_id")?;
    project_role::require_project_role(conn, project_id, actor, false).await?;
    if !project_role::channel_allowed(conn, row.try_get("channel_id")?, actor).await? {
        return Err(CoreError::NotFound);
    }
    Ok(LiveTicket {
        workspace_id: row.try_get("workspace_id")?,
        project_id,
    })
}

/// Attachments of the Ticket body, oldest first.
pub async fn list_body(
    pool: &PgPool,
    actor: Uuid,
    ticket_id: Uuid,
) -> Result<Vec<TicketAttachment>, CoreError> {
    let mut tx = pool.begin().await?;
    readable_ticket(&mut tx, actor, ticket_id).await?;
    let rows = sqlx::query(&format!(
        "SELECT {COLUMNS} {FROM}
         WHERE a.ticket_id = $1 AND a.comment_id IS NULL AND f.deleted_at IS NULL
         ORDER BY a.attached_at, a.id"
    ))
    .bind(ticket_id)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows.iter().map(attachment).collect::<Result<_, _>>()?)
}

/// Binds an uploaded file to the Ticket body. Project editors only; repeating the call for a
/// file already on this body returns it unchanged, and says nothing new to open screens.
pub async fn attach_body(
    pool: &PgPool,
    actor: Uuid,
    ticket_id: Uuid,
    file_id: Uuid,
) -> Result<(TicketAttachment, Option<ChangeNotice>), CoreError> {
    let mut tx = pool.begin().await?;
    let ticket = readable_ticket(&mut tx, actor, ticket_id).await?;
    project_role::require_project_role(&mut tx, ticket.project_id, actor, true).await?;
    let existing = sqlx::query(&format!(
        "SELECT {COLUMNS} {FROM}
         WHERE a.file_id = $1 AND a.ticket_id = $2 AND a.comment_id IS NULL"
    ))
    .bind(file_id)
    .bind(ticket_id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(row) = existing {
        return Ok((attachment(&row)?, None));
    }
    bind(
        &mut tx,
        actor,
        ticket.workspace_id,
        ticket_id,
        None,
        &[file_id],
    )
    .await?;
    let row = sqlx::query(&format!(
        "SELECT {COLUMNS} {FROM}
         WHERE a.file_id = $1"
    ))
    .bind(file_id)
    .fetch_one(&mut *tx)
    .await?;
    let change = work::comment_change(
        &mut tx,
        ticket.project_id,
        ticket_id,
        "ticket.file_attached",
    )
    .await?;
    tx.commit().await?;
    Ok((attachment(&row)?, Some(change)))
}

/// Removes a body attachment and its file. The person who attached it or a Project admin may;
/// comment attachments stay with their comment.
pub async fn detach_body(
    pool: &PgPool,
    store: Option<&ObjectStore>,
    actor: Uuid,
    ticket_id: Uuid,
    attachment_id: Uuid,
) -> Result<ChangeNotice, CoreError> {
    let mut tx = pool.begin().await?;
    let ticket = readable_ticket(&mut tx, actor, ticket_id).await?;
    let (_, role) =
        project_role::require_project_role(&mut tx, ticket.project_id, actor, true).await?;
    let row = sqlx::query(
        "SELECT file_id, comment_id, attached_by FROM penfii.ticket_attachments
         WHERE id = $1 AND ticket_id = $2 FOR UPDATE",
    )
    .bind(attachment_id)
    .bind(ticket_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(CoreError::NotFound)?;
    if row.try_get::<Option<Uuid>, _>("comment_id")?.is_some() {
        return Err(CoreError::conflict(
            "comment_attachment",
            "files on a comment stay with the comment",
        ));
    }
    if row.try_get::<Uuid, _>("attached_by")? != actor && !project_role::can_manage(&role) {
        return Err(CoreError::forbidden(
            "only the person who attached it or a project admin can remove it",
        ));
    }
    let file_id: Uuid = row.try_get("file_id")?;
    sqlx::query("DELETE FROM penfii.ticket_attachments WHERE id = $1")
        .bind(attachment_id)
        .execute(&mut *tx)
        .await?;
    let keys = files::retire(&mut tx, &[file_id]).await?;
    events::emit_simple(
        &mut tx,
        ticket.workspace_id,
        "FILE",
        file_id,
        "file.deleted",
        Some(actor),
        serde_json::json!({"ticketId": ticket_id}),
    )
    .await?;
    files::delete_objects(store, &keys).await?;
    let change = work::comment_change(
        &mut tx,
        ticket.project_id,
        ticket_id,
        "ticket.file_detached",
    )
    .await?;
    tx.commit().await?;
    Ok(change)
}

/// Binds files to a comment inside the transaction that creates it. The attachments live as long
/// as the comment: editing a comment leaves them, deleting it retires them.
pub(crate) async fn attach_to_comment(
    conn: &mut PgConnection,
    actor: Uuid,
    workspace_id: Uuid,
    ticket_id: Uuid,
    comment_id: Uuid,
    file_ids: &[Uuid],
) -> Result<(), CoreError> {
    bind(
        conn,
        actor,
        workspace_id,
        ticket_id,
        Some(comment_id),
        file_ids,
    )
    .await
}

/// Binds the uploads named when a Ticket is created; the caller has already checked that the
/// actor may create it.
pub(crate) async fn attach_to_new_ticket(
    conn: &mut PgConnection,
    actor: Uuid,
    workspace_id: Uuid,
    ticket_id: Uuid,
    file_ids: &[Uuid],
) -> Result<(), CoreError> {
    bind(conn, actor, workspace_id, ticket_id, None, file_ids).await
}

async fn bind(
    conn: &mut PgConnection,
    actor: Uuid,
    workspace_id: Uuid,
    ticket_id: Uuid,
    comment_id: Option<Uuid>,
    file_ids: &[Uuid],
) -> Result<(), CoreError> {
    let mut seen = Vec::new();
    for file_id in file_ids {
        if seen.contains(file_id) {
            continue;
        }
        seen.push(*file_id);
        files::claim_unbound(conn, actor, workspace_id, *file_id).await?;
        sqlx::query(
            "INSERT INTO penfii.ticket_attachments (id, ticket_id, comment_id, file_id, attached_by)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(Uuid::new_v4())
        .bind(ticket_id)
        .bind(comment_id)
        .bind(file_id)
        .bind(actor)
        .execute(&mut *conn)
        .await?;
        events::emit_simple(
            conn,
            workspace_id,
            "FILE",
            *file_id,
            "file.attached",
            Some(actor),
            serde_json::json!({"ticketId": ticket_id, "commentId": comment_id}),
        )
        .await?;
    }
    Ok(())
}

/// Attachments of the given comments, grouped by comment. Nothing while the Ticket is not live.
pub(crate) async fn for_comments(
    conn: &mut PgConnection,
    ticket_id: Uuid,
    comment_ids: &[Uuid],
) -> Result<HashMap<Uuid, Vec<TicketAttachment>>, CoreError> {
    let mut out: HashMap<Uuid, Vec<TicketAttachment>> = HashMap::new();
    if comment_ids.is_empty() {
        return Ok(out);
    }
    let rows = sqlx::query(&format!(
        "SELECT {COLUMNS} FROM penfii.ticket_attachments a
         JOIN penfii.files f ON f.id = a.file_id AND f.deleted_at IS NULL
         JOIN penfii.tickets t ON t.id = a.ticket_id
              AND t.archived_at IS NULL AND t.trashed_at IS NULL AND t.purged_at IS NULL
         WHERE a.ticket_id = $1 AND a.comment_id = ANY($2)
         ORDER BY a.attached_at, a.id"
    ))
    .bind(ticket_id)
    .bind(comment_ids)
    .fetch_all(&mut *conn)
    .await?;
    for row in &rows {
        let item = attachment(row)?;
        if let Some(comment) = item.comment_id {
            out.entry(comment).or_default().push(item);
        }
    }
    Ok(out)
}

/// What a download needs once the person's right to the Ticket is checked.
pub(crate) struct Located {
    pub attachment: MessageAttachment,
    pub storage_key: String,
}

/// Resolves an attachment id that belongs to a Ticket the person can open now. `None` when the id
/// is not a Ticket attachment at all (it may be a message attachment).
pub(crate) async fn locate(
    conn: &mut PgConnection,
    actor: Uuid,
    attachment_id: Uuid,
) -> Result<Option<Located>, CoreError> {
    let Some(row) = sqlx::query(
        "SELECT a.id, a.ticket_id, a.file_id, f.filename, f.byte_size, f.detected_mime, f.state,
                f.storage_key, f.deleted_at
         FROM penfii.ticket_attachments a JOIN penfii.files f ON f.id = a.file_id
         WHERE a.id = $1",
    )
    .bind(attachment_id)
    .fetch_optional(&mut *conn)
    .await?
    else {
        return Ok(None);
    };
    readable_ticket(conn, actor, row.try_get("ticket_id")?).await?;
    if row
        .try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("deleted_at")?
        .is_some()
        || row.try_get::<String, _>("state")? != "AVAILABLE"
    {
        return Err(CoreError::NotFound);
    }
    Ok(Some(Located {
        attachment: MessageAttachment {
            id: row.try_get("id")?,
            file_id: row.try_get("file_id")?,
            filename: row.try_get("filename")?,
            byte_size: row.try_get("byte_size")?,
            mime: row.try_get("detected_mime")?,
            state: row.try_get("state")?,
        },
        storage_key: row.try_get("storage_key")?,
    }))
}

/// Purge: drops every attachment of the Ticket (body and comments), retires the files and
/// returns the object keys. The caller removes the objects before it commits.
pub(crate) async fn retire_all(
    conn: &mut PgConnection,
    ticket_id: Uuid,
    workspace_id: Uuid,
    actor: Uuid,
) -> Result<Vec<String>, CoreError> {
    let file_ids: Vec<Uuid> = sqlx::query_scalar(
        "DELETE FROM penfii.ticket_attachments WHERE ticket_id = $1 RETURNING file_id",
    )
    .bind(ticket_id)
    .fetch_all(&mut *conn)
    .await?;
    retire_files(conn, workspace_id, actor, &file_ids).await
}

/// Deleting a comment drops its attachments the same way.
pub(crate) async fn retire_comment(
    conn: &mut PgConnection,
    comment_id: Uuid,
    workspace_id: Uuid,
    actor: Uuid,
) -> Result<Vec<String>, CoreError> {
    let file_ids: Vec<Uuid> = sqlx::query_scalar(
        "DELETE FROM penfii.ticket_attachments WHERE comment_id = $1 RETURNING file_id",
    )
    .bind(comment_id)
    .fetch_all(&mut *conn)
    .await?;
    retire_files(conn, workspace_id, actor, &file_ids).await
}

async fn retire_files(
    conn: &mut PgConnection,
    workspace_id: Uuid,
    actor: Uuid,
    file_ids: &[Uuid],
) -> Result<Vec<String>, CoreError> {
    let keys = files::retire(conn, file_ids).await?;
    for file_id in file_ids {
        events::emit_simple(
            conn,
            workspace_id,
            "FILE",
            *file_id,
            "file.deleted",
            Some(actor),
            serde_json::json!({}),
        )
        .await?;
    }
    Ok(keys)
}
