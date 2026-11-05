//! Where a file is bound, and the rules every attach path shares.
//!
//! A file belongs to at most one message, one Ticket attachment, one Doc comment, or any number of
//! Doc pages that name it. Chat, Docs and Tickets all ask here instead of each repeating "is it still unbound?",
//! so a new place that carries files only adds its table to the two queries below.

use std::collections::HashSet;

use sqlx::PgConnection;
use uuid::Uuid;

use super::{ObjectStore, StoredFile, load_file_for_update};
use crate::error::CoreError;

/// Files that sit on a message, in a Doc page or on a Ticket.
pub(crate) async fn bound_anywhere(
    conn: &mut PgConnection,
    ids: &[Uuid],
) -> Result<HashSet<Uuid>, CoreError> {
    if ids.is_empty() {
        return Ok(HashSet::new());
    }
    let rows: Vec<Uuid> = sqlx::query_scalar(
        "SELECT file_id FROM penfii.message_attachments WHERE file_id = ANY($1)
         UNION SELECT file_id FROM penfii.doc_page_files WHERE file_id = ANY($1)
         UNION SELECT file_id FROM penfii.ticket_attachments WHERE file_id = ANY($1)
         UNION SELECT file_id FROM penfii.doc_comment_files WHERE file_id = ANY($1)",
    )
    .bind(ids)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().collect())
}

/// Files that are not free for a Doc page to take: a message or a Ticket already owns them.
pub(crate) async fn bound_outside_docs(
    conn: &mut PgConnection,
    file_id: Uuid,
) -> Result<bool, CoreError> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM penfii.message_attachments WHERE file_id = $1)
             OR EXISTS(SELECT 1 FROM penfii.ticket_attachments WHERE file_id = $1)
             OR EXISTS(SELECT 1 FROM penfii.doc_comment_files WHERE file_id = $1)",
    )
    .bind(file_id)
    .fetch_one(&mut *conn)
    .await?)
}

/// Locks a finished upload so the caller may bind it: the actor's own file in this Workspace,
/// `AVAILABLE`, not deleted and not bound anywhere yet.
pub(crate) async fn claim_unbound(
    conn: &mut PgConnection,
    actor_id: Uuid,
    workspace_id: Uuid,
    file_id: Uuid,
) -> Result<StoredFile, CoreError> {
    let Some(row) = load_file_for_update(conn, file_id).await? else {
        return Err(CoreError::validation("file is not available"));
    };
    let file = row.stored();
    if file.created_by != actor_id
        || file.workspace_id != workspace_id
        || file.state != "AVAILABLE"
        || file.deleted
    {
        return Err(CoreError::validation("file is not available"));
    }
    if !bound_anywhere(conn, &[file_id]).await?.is_empty() {
        return Err(CoreError::conflict(
            "file_attached",
            "file is already attached",
        ));
    }
    Ok(file)
}

/// Marks files deleted for good (a later upload of the same key can never revive them), cancels
/// their intake work and returns the object keys the caller must remove from storage.
pub(crate) async fn retire(
    conn: &mut PgConnection,
    ids: &[Uuid],
) -> Result<Vec<String>, CoreError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let keys: Vec<String> = sqlx::query_scalar(
        "UPDATE penfii.files
         SET deleted_at = now(), generation = generation + 1
         WHERE id = ANY($1) AND deleted_at IS NULL
         RETURNING storage_key",
    )
    .bind(ids)
    .fetch_all(&mut *conn)
    .await?;
    sqlx::query(
        "UPDATE penfii.file_processing_tasks
         SET state = 'CANCELED', lease_token = NULL, lease_until = NULL
         WHERE file_id = ANY($1) AND state IN ('READY', 'RUNNING')",
    )
    .bind(ids)
    .execute(&mut *conn)
    .await?;
    Ok(keys)
}

/// Removes the objects of retired files. Callers that can still roll back (purge, removing an
/// attachment) run it before they commit: a failure aborts the change so it can be repeated, and
/// deleting an object that is already gone succeeds, so no object outlives its row.
pub(crate) async fn delete_objects(
    store: Option<&ObjectStore>,
    keys: &[String],
) -> Result<(), CoreError> {
    if let Some(store) = store {
        for key in keys {
            store.delete(key).await.map_err(CoreError::Internal)?;
        }
    }
    Ok(())
}
