//! File metadata in PostgreSQL and bytes in object storage.
//! A late intake result cannot revive a deleted or superseded generation.

mod binding;
mod store;

use penfii_contracts::MessageAttachment;
use serde_json::json;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::access;
use crate::error::CoreError;
use crate::limits::Limits;

pub(crate) use binding::{
    bound_anywhere, bound_outside_docs, claim_unbound, delete_objects, retire,
};
pub use store::ObjectStore;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct StoredFile {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub storage_key: String,
    pub filename: String,
    pub byte_size: i64,
    pub detected_mime: Option<String>,
    pub state: String,
    pub generation: i64,
    pub created_by: Uuid,
    pub deleted: bool,
}

pub async fn begin_upload(
    pool: &PgPool,
    limits: &Limits,
    actor_id: Uuid,
    workspace_id: Uuid,
    filename: &str,
    byte_size: i64,
) -> Result<StoredFile, CoreError> {
    let max = file_limit(limits)?;
    if byte_size <= 0 || byte_size > max {
        return Err(CoreError::validation(
            "file size is outside the configured limit",
        ));
    }
    let filename = clean_filename(filename)?;
    let mut tx = pool.begin().await?;
    access::require_member(&mut tx, workspace_id, actor_id).await?;
    let id = Uuid::new_v4();
    let generation = 1_i64;
    let storage_key = format!("workspaces/{workspace_id}/files/{id}/g{generation}");
    sqlx::query(
        "INSERT INTO penfii.files
            (id, workspace_id, storage_key, filename, byte_size, state, generation, created_by)
         VALUES ($1, $2, $3, $4, $5, 'UPLOADING', $6, $7)",
    )
    .bind(id)
    .bind(workspace_id)
    .bind(&storage_key)
    .bind(&filename)
    .bind(byte_size)
    .bind(generation)
    .bind(actor_id)
    .execute(&mut *tx)
    .await?;
    let file = load_file(&mut tx, id).await?.ok_or(CoreError::NotFound)?;
    tx.commit().await?;
    Ok(file)
}

pub async fn finish_upload(
    pool: &PgPool,
    store: &ObjectStore,
    limits: &Limits,
    actor_id: Uuid,
    file_id: Uuid,
    bytes: &[u8],
) -> Result<StoredFile, CoreError> {
    let max = file_limit(limits)?;
    let len = i64::try_from(bytes.len()).unwrap_or(i64::MAX);
    if len <= 0 || len > max {
        return Err(CoreError::validation(
            "file size is outside the configured limit",
        ));
    }
    let mut tx = pool.begin().await?;
    let Some(file) = load_file_for_update(&mut tx, file_id).await? else {
        return Err(CoreError::NotFound);
    };
    if file.created_by != actor_id || file.deleted_at_present() {
        return Err(CoreError::NotFound);
    }
    if file.state != "UPLOADING" || file.byte_size != len {
        return Err(CoreError::conflict(
            "stale_upload",
            "upload is no longer current",
        ));
    }
    let generation = file.generation;
    let key = file.storage_key.clone();
    let filename = file.filename.clone();
    tx.commit().await?;

    store.put(&key, bytes).await.map_err(CoreError::Internal)?;

    let mut tx = pool.begin().await?;
    let Some(current) = load_file_for_update(&mut tx, file_id).await? else {
        let _ = store.delete(&key).await;
        return Err(CoreError::NotFound);
    };
    if current.created_by != actor_id
        || current.deleted_at_present()
        || current.generation != generation
        || current.state != "UPLOADING"
    {
        // The object key is per file and generation. When another request of the same upload
        // finished it while this one was writing, that request owns the object now: deleting it
        // would leave a finished file without its bytes. Only an upload that was deleted or
        // replaced leaves an object nobody refers to.
        let finished_by_another = current.created_by == actor_id
            && !current.deleted_at_present()
            && current.generation == generation;
        if !finished_by_another {
            let _ = store.delete(&key).await;
        }
        return Err(CoreError::conflict(
            "stale_upload",
            "upload was deleted or replaced",
        ));
    }
    let mime = accept_bytes(&filename, bytes);
    let next = if mime.accepted {
        "QUARANTINED"
    } else {
        "REJECTED"
    };
    sqlx::query(
        "UPDATE penfii.files
         SET state = $2, detected_mime = $3
         WHERE id = $1 AND generation = $4 AND deleted_at IS NULL AND state = 'UPLOADING'",
    )
    .bind(file_id)
    .bind(next)
    .bind(mime.detected)
    .bind(generation)
    .execute(&mut *tx)
    .await?;
    if mime.accepted {
        settle_intake(&mut tx, file_id, generation, &key).await?;
    }
    let stored = load_file(&mut tx, file_id)
        .await?
        .ok_or(CoreError::NotFound)?;
    tx.commit().await?;
    Ok(stored)
}

pub async fn delete_upload(
    pool: &PgPool,
    store: Option<&ObjectStore>,
    actor_id: Uuid,
    file_id: Uuid,
) -> Result<(), CoreError> {
    let mut tx = pool.begin().await?;
    let Some(file) = load_file_for_update(&mut tx, file_id).await? else {
        return Err(CoreError::NotFound);
    };
    if file.created_by != actor_id {
        return Err(CoreError::NotFound);
    }
    if !bound_anywhere(&mut tx, &[file_id]).await?.is_empty() {
        return Err(CoreError::conflict(
            "file_attached",
            "attached files stay with their message, page or ticket",
        ));
    }
    let keys = retire(&mut tx, &[file_id]).await?;
    tx.commit().await?;
    delete_objects(store, &keys).await
}

/// Removes uploads nobody ever attached: a form closed before sending, a tab that died mid-upload.
/// A client deletes its own leftovers when it can; this catches the ones it could not. Returns how
/// many files were retired.
pub async fn sweep_unbound(
    pool: &PgPool,
    store: Option<&ObjectStore>,
    older_than_seconds: i64,
) -> Result<usize, CoreError> {
    let mut tx = pool.begin().await?;
    let ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT f.id FROM penfii.files f
         WHERE f.deleted_at IS NULL
           AND f.created_at < now() - make_interval(secs => $1::float8)
           AND NOT EXISTS (SELECT 1 FROM penfii.message_attachments WHERE file_id = f.id)
           AND NOT EXISTS (SELECT 1 FROM penfii.doc_page_files WHERE file_id = f.id)
           AND NOT EXISTS (SELECT 1 FROM penfii.ticket_attachments WHERE file_id = f.id)
           AND NOT EXISTS (SELECT 1 FROM penfii.doc_comment_files WHERE file_id = f.id)
           AND NOT EXISTS (SELECT 1 FROM penfii.message_version_files WHERE file_id = f.id)
           AND NOT EXISTS (SELECT 1 FROM penfii.doc_version_files WHERE file_id = f.id)
         ORDER BY f.created_at
         LIMIT 200
         FOR UPDATE OF f SKIP LOCKED",
    )
    .bind(older_than_seconds as f64)
    .fetch_all(&mut *tx)
    .await?;
    if ids.is_empty() {
        return Ok(0);
    }
    let keys = retire(&mut tx, &ids).await?;
    delete_objects(store, &keys).await?;
    tx.commit().await?;
    Ok(ids.len())
}

/// Runs [`sweep_unbound`] every ten minutes. `PENFII_UNBOUND_UPLOAD_GRACE_SECONDS` (default one
/// day) is how long an unattached upload may wait for its form to be sent.
pub fn spawn_sweeper(pool: PgPool, store: Option<ObjectStore>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let grace = std::env::var("PENFII_UNBOUND_UPLOAD_GRACE_SECONDS")
            .ok()
            .and_then(|raw| raw.trim().parse::<i64>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(24 * 3600);
        loop {
            match sweep_unbound(&pool, store.as_ref(), grace).await {
                Ok(0) => {}
                Ok(count) => tracing::info!(count, "unattached uploads removed"),
                Err(error) => tracing::warn!(?error, "unattached upload sweep failed"),
            }
            tokio::time::sleep(std::time::Duration::from_secs(600)).await;
        }
    })
}

/// Resolves `/v1/files/{id}/content` to the stored object. A file that belongs to a Doc page
/// is readable by whoever can read that Doc and by nobody else, its uploader included
/// once they lost access. Any other file is its uploader's own pending upload.
pub async fn locate_owned(
    pool: &PgPool,
    actor_id: Uuid,
    file_id: Uuid,
) -> Result<FileSource, CoreError> {
    let mut conn = pool.acquire().await?;
    let in_knowledge = crate::visibility::attached_to_knowledge(&mut conn, &[file_id]).await?;
    let file = if in_knowledge.contains(&file_id) {
        let readable = crate::visibility::knowledge_files(&mut conn, actor_id, &[file_id]).await?;
        let file = load_file(&mut conn, file_id)
            .await?
            .ok_or(CoreError::NotFound)?;
        if !readable.contains(&file_id) || file.state != "AVAILABLE" || file.deleted {
            return Err(CoreError::NotFound);
        }
        file
    } else {
        drop(conn);
        load_available_owned(pool, actor_id, file_id).await?
    };
    Ok(FileSource {
        filename: file.filename,
        mime: file.detected_mime,
        storage_key: file.storage_key,
    })
}

/// A file the actor may read: what the response needs to name it, and where its bytes are.
pub struct FileSource {
    pub filename: String,
    pub mime: Option<String>,
    pub storage_key: String,
}

/// Resolves `/v1/attachments/{id}/content` to the stored object for a message or Ticket attachment.
pub async fn locate_attachment(
    pool: &PgPool,
    actor_id: Uuid,
    attachment_id: Uuid,
) -> Result<FileSource, CoreError> {
    let mut tx = pool.begin().await?;
    let Some(row) = sqlx::query_as::<_, AttachmentRow>(
        "SELECT a.id, a.file_id, f.filename, f.byte_size, f.detected_mime, f.state,
                f.storage_key, f.deleted_at, m.conversation_id, m.deleted_at AS message_deleted_at
         FROM penfii.message_attachments a
         JOIN penfii.files f ON f.id = a.file_id
         JOIN penfii.messages m ON m.id = a.message_id
         WHERE a.id = $1",
    )
    .bind(attachment_id)
    .fetch_optional(&mut *tx)
    .await?
    else {
        // Not a message attachment: it may belong to a Ticket the person can open.
        let Some(located) = crate::ticket_files::locate(&mut tx, actor_id, attachment_id).await?
        else {
            // Or a file on a Doc comment, which anyone who can read the page can open.
            let source = crate::docs::locate_comment_file(&mut tx, actor_id, attachment_id)
                .await?
                .ok_or(CoreError::NotFound)?;
            tx.commit().await?;
            return Ok(source);
        };
        tx.commit().await?;
        return Ok(FileSource {
            filename: located.attachment.filename,
            mime: located.attachment.mime,
            storage_key: located.storage_key,
        });
    };
    access::require_read(&mut tx, actor_id, row.conversation_id).await?;
    if row.deleted_at.is_some() || row.message_deleted_at.is_some() || row.state != "AVAILABLE" {
        return Err(CoreError::NotFound);
    }
    tx.commit().await?;
    Ok(FileSource {
        filename: row.filename,
        mime: row.detected_mime,
        storage_key: row.storage_key,
    })
}

pub async fn attach_to_message(
    conn: &mut PgConnection,
    limits: &Limits,
    actor_id: Uuid,
    workspace_id: Uuid,
    message_id: Uuid,
    file_ids: &[Uuid],
) -> Result<(), CoreError> {
    if file_ids.is_empty() {
        return Ok(());
    }
    let max_count = limits.message_file_max_count.ok_or_else(|| {
        CoreError::conflict(
            "file_limit_unconfigured",
            "message file count limit is not configured",
        )
    })?;
    let max_total = limits.message_file_max_bytes.ok_or_else(|| {
        CoreError::conflict(
            "file_limit_unconfigured",
            "message file size limit is not configured",
        )
    })?;
    let mut unique = Vec::new();
    for id in file_ids {
        if !unique.contains(id) {
            unique.push(*id);
        }
    }
    if i64::try_from(unique.len()).unwrap_or(i64::MAX) > max_count {
        return Err(CoreError::validation("too many files on one message"));
    }
    let mut total = 0_i64;
    for file_id in unique {
        let file = claim_unbound(conn, actor_id, workspace_id, file_id).await?;
        total = total.saturating_add(file.byte_size);
        if total > max_total {
            return Err(CoreError::validation("files exceed the message total"));
        }
        sqlx::query(
            "INSERT INTO penfii.message_attachments (id, message_id, file_id) VALUES ($1, $2, $3)",
        )
        .bind(Uuid::new_v4())
        .bind(message_id)
        .bind(file_id)
        .execute(&mut *conn)
        .await?;
        crate::events::emit_simple(
            &mut *conn,
            workspace_id,
            "FILE",
            file_id,
            "file.attached",
            Some(actor_id),
            serde_json::json!({"messageId": message_id}),
        )
        .await?;
    }
    Ok(())
}

pub async fn copy_version_files(
    conn: &mut PgConnection,
    message_id: Uuid,
    version: i64,
) -> Result<(), CoreError> {
    sqlx::query(
        "INSERT INTO penfii.message_version_files (message_id, version, file_id)
         SELECT message_id, $2, file_id FROM penfii.message_attachments WHERE message_id = $1",
    )
    .bind(message_id)
    .bind(version)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

pub async fn attachments_for(
    conn: &mut PgConnection,
    message_ids: &[Uuid],
) -> Result<Vec<(Uuid, MessageAttachment)>, CoreError> {
    if message_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query_as::<_, (Uuid, Uuid, Uuid, String, i64, Option<String>, String)>(
        "SELECT a.message_id, a.id, a.file_id, f.filename, f.byte_size, f.detected_mime, f.state
         FROM penfii.message_attachments a
         JOIN penfii.files f ON f.id = a.file_id
         WHERE a.message_id = ANY($1) AND f.deleted_at IS NULL
         ORDER BY a.message_id, f.created_at, a.id",
    )
    .bind(message_ids)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(message_id, id, file_id, filename, byte_size, mime, state)| {
                (
                    message_id,
                    MessageAttachment {
                        id,
                        file_id,
                        filename,
                        byte_size,
                        mime,
                        state,
                    },
                )
            },
        )
        .collect())
}

async fn settle_intake(
    conn: &mut PgConnection,
    file_id: Uuid,
    generation: i64,
    storage_key: &str,
) -> Result<(), CoreError> {
    let task_id = Uuid::new_v4();
    let lease = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO penfii.file_processing_tasks
            (id, file_id, file_generation, storage_key, task_kind, state)
         VALUES ($1, $2, $3, $4, 'INTAKE', 'READY')",
    )
    .bind(task_id)
    .bind(file_id)
    .bind(generation)
    .bind(storage_key)
    .execute(&mut *conn)
    .await?;
    let claimed = sqlx::query(
        "UPDATE penfii.file_processing_tasks
         SET state = 'RUNNING', lease_token = $2, lease_until = now() + interval '5 minutes', attempts = 1
         WHERE id = $1 AND state = 'READY' AND file_generation = $3",
    )
    .bind(task_id)
    .bind(lease)
    .bind(generation)
    .execute(&mut *conn)
    .await?;
    if claimed.rows_affected() != 1 {
        return Err(CoreError::conflict(
            "stale_upload",
            "intake could not start",
        ));
    }
    sqlx::query(
        "INSERT INTO penfii.file_processing_results (task_id, lease_token, verdict, artifacts)
         VALUES ($1, $2, 'CLEAN', $3)",
    )
    .bind(task_id)
    .bind(lease)
    .bind(json!({ "intake": "structural" }))
    .execute(&mut *conn)
    .await?;
    let applied = sqlx::query(
        "UPDATE penfii.files
         SET state = 'AVAILABLE'
         WHERE id = $1 AND generation = $2 AND deleted_at IS NULL AND state = 'QUARANTINED'",
    )
    .bind(file_id)
    .bind(generation)
    .execute(&mut *conn)
    .await?;
    if applied.rows_affected() != 1 {
        return Err(CoreError::conflict(
            "stale_upload",
            "file generation changed",
        ));
    }
    sqlx::query(
        "UPDATE penfii.file_processing_tasks
         SET state = 'DONE', lease_token = NULL, lease_until = NULL
         WHERE id = $1 AND lease_token = $2 AND state = 'RUNNING' AND lease_until > now()",
    )
    .bind(task_id)
    .bind(lease)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

struct MimeDecision {
    accepted: bool,
    detected: Option<String>,
}

fn accept_bytes(filename: &str, bytes: &[u8]) -> MimeDecision {
    let extension = filename
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let image_ext = matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "webp" | "gif");
    let detected = detect_mime(&extension, bytes);
    if image_ext
        && detected
            .as_deref()
            .is_none_or(|mime| !mime.starts_with("image/"))
    {
        return MimeDecision {
            accepted: false,
            detected: None,
        };
    }
    MimeDecision {
        accepted: true,
        detected: detected.or(Some("application/octet-stream".into())),
    }
}

/// The type of a file, decided by what its bytes are (the name only separates formats whose bytes
/// do not tell, like Ogg audio from Ogg video, and CSV, which has no signature). What the browser
/// sent is never used. `None` is an unknown type, served as a download.
fn detect_mime(extension: &str, bytes: &[u8]) -> Option<String> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        return Some("image/png".into());
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("image/jpeg".into());
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif".into());
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("image/webp".into());
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        return Some("audio/wav".into());
    }
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        return Some(
            match &bytes[8..12] {
                b"M4A " | b"M4B " => "audio/mp4",
                b"qt  " => "video/quicktime",
                _ => "video/mp4",
            }
            .into(),
        );
    }
    if bytes.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        return Some("video/webm".into());
    }
    if bytes.starts_with(b"OggS") {
        return Some(
            if extension == "ogv" {
                "video/ogg"
            } else {
                "audio/ogg"
            }
            .into(),
        );
    }
    if bytes.starts_with(b"ID3")
        || (bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] & 0xE0 == 0xE0)
    {
        return Some("audio/mpeg".into());
    }
    if bytes.starts_with(b"%PDF-") {
        return Some("application/pdf".into());
    }
    if extension == "csv"
        && std::str::from_utf8(&bytes[..bytes.len().min(64 * 1024)])
            .is_ok_and(|_| !bytes.contains(&0))
    {
        return Some("text/csv".into());
    }
    None
}

fn clean_filename(raw: &str) -> Result<String, CoreError> {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or("").trim();
    if base.is_empty() || base == "." || base == ".." || base.len() > 200 || base.contains('\u{0}')
    {
        return Err(CoreError::validation("filename is invalid"));
    }
    let extension = base.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    if matches!(extension.as_str(), "env" | "pem" | "key") {
        return Err(CoreError::validation("this file type is blocked"));
    }
    Ok(base.to_string())
}

fn file_limit(limits: &Limits) -> Result<i64, CoreError> {
    limits.file_max_bytes.ok_or_else(|| {
        CoreError::conflict(
            "file_limit_unconfigured",
            "file size limit is not configured",
        )
    })
}

#[derive(sqlx::FromRow)]
struct FileRow {
    id: Uuid,
    workspace_id: Uuid,
    storage_key: String,
    filename: String,
    byte_size: i64,
    detected_mime: Option<String>,
    state: String,
    generation: i64,
    created_by: Uuid,
    deleted_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl FileRow {
    fn deleted_at_present(&self) -> bool {
        self.deleted_at.is_some()
    }

    fn stored(self) -> StoredFile {
        StoredFile {
            id: self.id,
            workspace_id: self.workspace_id,
            storage_key: self.storage_key,
            filename: self.filename,
            byte_size: self.byte_size,
            detected_mime: self.detected_mime,
            state: self.state,
            generation: self.generation,
            created_by: self.created_by,
            deleted: self.deleted_at.is_some(),
        }
    }
}

const FILE_COLUMNS: &str = "id, workspace_id, storage_key, filename, byte_size, detected_mime, state, generation, created_by, deleted_at";

async fn load_file(conn: &mut PgConnection, id: Uuid) -> Result<Option<StoredFile>, CoreError> {
    let row = sqlx::query_as::<_, FileRow>(&format!(
        "SELECT {FILE_COLUMNS} FROM penfii.files WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.map(FileRow::stored))
}

async fn load_file_for_update(
    conn: &mut PgConnection,
    id: Uuid,
) -> Result<Option<FileRow>, CoreError> {
    let row = sqlx::query_as::<_, FileRow>(&format!(
        "SELECT {FILE_COLUMNS} FROM penfii.files WHERE id = $1 FOR UPDATE"
    ))
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row)
}

async fn load_available_owned(
    pool: &PgPool,
    actor_id: Uuid,
    file_id: Uuid,
) -> Result<StoredFile, CoreError> {
    let mut tx = pool.begin().await?;
    let Some(file) = load_file(&mut tx, file_id).await? else {
        return Err(CoreError::NotFound);
    };
    if file.created_by != actor_id || file.state != "AVAILABLE" {
        return Err(CoreError::NotFound);
    }
    let attached = !bound_anywhere(&mut tx, &[file_id]).await?.is_empty();
    let deleted = sqlx::query_scalar::<_, bool>(
        "SELECT deleted_at IS NOT NULL FROM penfii.files WHERE id = $1",
    )
    .bind(file_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    if attached || deleted {
        return Err(CoreError::NotFound);
    }
    Ok(file)
}

#[derive(sqlx::FromRow)]
struct AttachmentRow {
    filename: String,
    detected_mime: Option<String>,
    state: String,
    storage_key: String,
    deleted_at: Option<chrono::DateTime<chrono::Utc>>,
    conversation_id: Uuid,
    message_deleted_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[cfg(test)]
mod tests {
    use super::{accept_bytes, clean_filename, detect_mime};

    #[test]
    fn image_magic_and_blocked_names() {
        assert_eq!(
            detect_mime("png", b"\x89PNG\r\n").as_deref(),
            Some("image/png")
        );
        assert!(accept_bytes("photo.png", b"\x89PNG\r\n").accepted);
        assert!(!accept_bytes("photo.png", b"not an image").accepted);
        assert!(clean_filename("notes.txt").is_ok());
        assert!(clean_filename("../secret.pem").is_err());
        assert!(clean_filename("folder/report.pdf").unwrap() == "report.pdf");
    }

    #[test]
    fn media_is_told_by_its_bytes_and_active_content_stays_unknown() {
        let mime = |ext: &str, bytes: &[u8]| detect_mime(ext, bytes);
        assert_eq!(
            mime("mp4", b"\0\0\0\x18ftypisom\0\0").as_deref(),
            Some("video/mp4")
        );
        assert_eq!(
            mime("m4a", b"\0\0\0\x18ftypM4A \0\0").as_deref(),
            Some("audio/mp4")
        );
        assert_eq!(
            mime("webm", &[0x1A, 0x45, 0xDF, 0xA3, 0]).as_deref(),
            Some("video/webm")
        );
        assert_eq!(mime("ogg", b"OggS\0").as_deref(), Some("audio/ogg"));
        assert_eq!(mime("ogv", b"OggS\0").as_deref(), Some("video/ogg"));
        assert_eq!(mime("mp3", b"ID3\x04").as_deref(), Some("audio/mpeg"));
        assert_eq!(
            mime("wav", b"RIFF\0\0\0\0WAVEfmt ").as_deref(),
            Some("audio/wav")
        );
        assert_eq!(mime("pdf", b"%PDF-1.7").as_deref(), Some("application/pdf"));
        assert_eq!(
            mime("csv", "a,b\n1,2\n".as_bytes()).as_deref(),
            Some("text/csv")
        );
        assert_eq!(mime("csv", b"\0\x01\x02"), None);
        // HTML and SVG are never recognised, whatever they are called.
        assert_eq!(mime("html", b"<!doctype html><script>1</script>"), None);
        assert_eq!(
            mime("svg", b"<svg xmlns='http://www.w3.org/2000/svg'/>"),
            None
        );
        // A video that claims to be a picture is refused.
        assert!(!accept_bytes("clip.png", b"\0\0\0\x18ftypisom\0\0").accepted);
    }
}
