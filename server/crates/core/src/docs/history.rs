//! Saving a page and its Versions.
//!
//! Two things are kept apart. The *saved document* is `doc_pages` (content, title, `revision_no`);
//! a Save replaces it and bumps `revision_no`, the token Save and rename compare. A *Version* is a
//! row of `doc_versions`, written only when a person chooses Create Version; it freezes the saved
//! document at that moment and is never changed afterwards. `version_no` counts Versions and has
//! nothing to do with `revision_no`. Restoring is not a command here: the client reads a Version and
//! offers it as the next draft, and the person saves it like any other edit.

use serde_json::{Value, json};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use super::{
    DocRights, clean_title, collab, content, emit_doc, load_page, lock_readable, materialize,
    page_json, time,
};
use crate::error::CoreError;
use crate::events;
use crate::knowledge::KnowledgeConfig;

const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 200;

fn conflict(current_revision: i64) -> CoreError {
    CoreError::conflict_extra(
        "version_conflict",
        "the page was saved by someone else since you opened it",
        serde_json::from_value(json!({"currentRevision": current_revision})).unwrap_or_default(),
    )
}

fn require_edit(rights: DocRights) -> Result<(), CoreError> {
    if rights.edit {
        Ok(())
    } else {
        Err(CoreError::forbidden("you cannot edit this page"))
    }
}

/// Replaces the saved document with the person's draft. The draft is the whole title and content;
/// nothing is merged. `expected_revision` is the `revisionNo` the draft started from.
pub async fn save(
    pool: &PgPool,
    config: &KnowledgeConfig,
    actor: Uuid,
    page_id: Uuid,
    expected_revision: i64,
    title: &str,
    candidate: &Value,
) -> Result<Value, CoreError> {
    let title = clean_title(title)?;
    let max = usize::try_from(config.doc_max_content_bytes).unwrap_or(usize::MAX);
    let mut tx = pool.begin().await?;
    let (workspace, rights) = lock_readable(&mut tx, actor, page_id).await?;
    require_edit(rights)?;
    let current =
        sqlx::query("SELECT title, content_hash, revision_no FROM penfii.doc_pages WHERE id=$1")
            .bind(page_id)
            .fetch_one(&mut *tx)
            .await?;
    let old_title: String = current.try_get("title")?;
    let old_hash: String = current.try_get("content_hash")?;
    let revision: i64 = current.try_get("revision_no")?;
    if revision != expected_revision {
        return Err(conflict(revision));
    }

    // A file may be named only if it was attached to this page on purpose (upload + attach) or is
    // kept by a Version of it (a Restore can bring such a file back).
    let attached: std::collections::HashSet<Uuid> = sqlx::query_scalar(
        "SELECT file_id FROM penfii.doc_page_files WHERE page_id=$1
             UNION SELECT file_id FROM penfii.doc_version_files WHERE page_id=$1",
    )
    .bind(page_id)
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .collect();
    let (_, first) = content::sanitize(candidate, max)
        .map_err(|e| CoreError::rule("doc_content_invalid", e.0))?;
    let filtered;
    let candidate = if first.files.iter().all(|f| attached.contains(f)) {
        candidate
    } else {
        filtered = content::filter_files(candidate, &|f| attached.contains(&f));
        &filtered
    };
    let (clean, found) = content::sanitize(candidate, max)
        .map_err(|e| CoreError::rule("doc_content_invalid", e.0))?;
    let hash = content::hash(&clean);
    if hash != old_hash || title != old_title {
        let revision: i64 = sqlx::query_scalar(
            "UPDATE penfii.doc_pages
             SET content=$2, content_hash=$3, title=$4, revision_no=revision_no+1,
                 materialized_at=now(), updated_at=now(), updated_by=$5
             WHERE id=$1 RETURNING revision_no",
        )
        .bind(page_id)
        .bind(&clean)
        .bind(&hash)
        .bind(&title)
        .bind(actor)
        .fetch_one(&mut *tx)
        .await?;
        if hash != old_hash {
            // The co-editing layer's working copy follows the saved document.
            collab::store::reseed(&mut tx, page_id, &clean).await?;
        }
        materialize::finish_write(
            &mut tx,
            config,
            page_id,
            workspace,
            revision,
            &found,
            Some(actor),
            json!({"source": "save", "editorIds": [actor]}),
        )
        .await?;
    }
    let page = load_page(&mut tx, page_id).await?;
    let value = page_json(&mut tx, actor, page, false).await?;
    tx.commit().await?;
    Ok(value)
}

/// Freezes the saved document as the next Version. Refused when the saved document equals the last
/// Version (title and content), so History holds no duplicates.
pub async fn create_version(
    pool: &PgPool,
    actor: Uuid,
    page_id: Uuid,
    expected_revision: i64,
) -> Result<Value, CoreError> {
    let mut tx = pool.begin().await?;
    let (workspace, rights) = lock_readable(&mut tx, actor, page_id).await?;
    require_edit(rights)?;
    let page = load_page(&mut tx, page_id).await?;
    if page.revision_no != expected_revision {
        return Err(conflict(page.revision_no));
    }
    if page.title.trim().is_empty() {
        return Err(CoreError::rule(
            "doc_title_required",
            "give the page a title (Edit, then Save) before creating a Version",
        ));
    }
    let hash: String = sqlx::query_scalar("SELECT content_hash FROM penfii.doc_pages WHERE id=$1")
        .bind(page_id)
        .fetch_one(&mut *tx)
        .await?;
    let last = sqlx::query(
        "SELECT version_no, title, content_hash FROM penfii.doc_versions
         WHERE page_id=$1 ORDER BY version_no DESC LIMIT 1",
    )
    .bind(page_id)
    .fetch_optional(&mut *tx)
    .await?;
    let next = match &last {
        Some(row) => {
            let same = row.try_get::<String, _>("content_hash")? == hash
                && row.try_get::<String, _>("title")? == page.title;
            if same {
                return Err(CoreError::rule(
                    "doc_version_unchanged",
                    "the saved page equals the latest Version",
                ));
            }
            row.try_get::<i32, _>("version_no")? + 1
        }
        None => 1,
    };
    sqlx::query(
        "INSERT INTO penfii.doc_versions(page_id,version_no,title,content,content_schema_version,content_hash,source_revision_no,created_by)
         SELECT id,$2,title,content,content_schema_version,content_hash,revision_no,$3
         FROM penfii.doc_pages WHERE id=$1",
    )
    .bind(page_id)
    .bind(next)
    .bind(actor)
    .execute(&mut *tx)
    .await?;
    // The files the Version names stay with the page while the Version exists, so History can show
    // them and a Restore can bring them back after the page itself stopped using them.
    let (_, found) = content::sanitize(&page.content, usize::MAX)
        .map_err(|e| CoreError::Internal(format!("stored content: {}", e.0)))?;
    for file in &found.files {
        sqlx::query(
            "INSERT INTO penfii.doc_version_files(page_id,version_no,file_id) VALUES($1,$2,$3)
             ON CONFLICT DO NOTHING",
        )
        .bind(page_id)
        .bind(next)
        .bind(file)
        .execute(&mut *tx)
        .await?;
    }
    events::emit(
        &mut tx,
        emit_doc(
            workspace,
            page_id,
            Some(page.revision_no),
            "doc.version_created",
            actor,
            json!({"versionNo": next}),
        ),
    )
    .await?;
    let value = version_json(&mut tx, page_id, next, false).await?;
    tx.commit().await?;
    Ok(value)
}

async fn version_json(
    conn: &mut sqlx::PgConnection,
    page_id: Uuid,
    version_no: i32,
    with_content: bool,
) -> Result<Value, CoreError> {
    let row = sqlx::query(
        "SELECT v.version_no, v.title, v.content, v.content_hash, v.source_revision_no, v.created_by,
                v.created_at, u.display_name,
                (v.content_hash = p.content_hash AND v.title = p.title) AS is_current
         FROM penfii.doc_versions v
         JOIN penfii.doc_pages p ON p.id = v.page_id
         JOIN penfii.users u ON u.id = v.created_by
         WHERE v.page_id=$1 AND v.version_no=$2",
    )
    .bind(page_id)
    .bind(version_no)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(CoreError::NotFound)?;
    Ok(version_row(&row, with_content)?)
}

fn version_row(row: &sqlx::postgres::PgRow, with_content: bool) -> Result<Value, sqlx::Error> {
    let mut value = json!({
        "versionNo": row.try_get::<i32, _>("version_no")?,
        "title": row.try_get::<String, _>("title")?,
        "sourceRevisionNo": row.try_get::<i64, _>("source_revision_no")?,
        "createdBy": row.try_get::<Uuid, _>("created_by")?,
        "createdByName": row.try_get::<String, _>("display_name")?,
        "createdAt": time(row.try_get("created_at")?),
        "isCurrent": row.try_get::<bool, _>("is_current")?,
    });
    if with_content {
        value["content"] = row.try_get::<Value, _>("content")?;
    }
    Ok(value)
}

/// The Versions of a page, newest first.
pub async fn list_versions(
    pool: &PgPool,
    actor: Uuid,
    page_id: Uuid,
    before: Option<i32>,
    limit: Option<i64>,
) -> Result<Value, CoreError> {
    let mut conn = pool.acquire().await?;
    if !super::rights_of(&mut conn, actor, page_id).await?.read {
        return Err(CoreError::NotFound);
    }
    load_page(&mut conn, page_id).await?;
    let limit = crate::paging::clamp_limit(limit, DEFAULT_LIMIT, MAX_LIMIT);
    let rows = sqlx::query(
        "SELECT v.version_no, v.title, v.content, v.content_hash, v.source_revision_no, v.created_by,
                v.created_at, u.display_name,
                (v.content_hash = p.content_hash AND v.title = p.title) AS is_current
         FROM penfii.doc_versions v
         JOIN penfii.doc_pages p ON p.id = v.page_id
         JOIN penfii.users u ON u.id = v.created_by
         WHERE v.page_id=$1 AND ($2::int IS NULL OR v.version_no < $2)
         ORDER BY v.version_no DESC LIMIT $3",
    )
    .bind(page_id)
    .bind(before)
    .bind(limit + 1)
    .fetch_all(&mut *conn)
    .await?;
    let more = rows.len() as i64 > limit;
    let items = rows
        .iter()
        .take(usize::try_from(limit).unwrap_or(0))
        .map(|r| version_row(r, false))
        .collect::<Result<Vec<_>, _>>()?;
    let next = more
        .then(|| items.last().and_then(|v| v["versionNo"].as_i64()))
        .flatten();
    Ok(json!({"items": items, "nextBefore": next}))
}

/// One Version with its content (what Restore reads).
pub async fn get_version(
    pool: &PgPool,
    actor: Uuid,
    page_id: Uuid,
    version_no: i32,
) -> Result<Value, CoreError> {
    let mut conn = pool.acquire().await?;
    if !super::rights_of(&mut conn, actor, page_id).await?.read {
        return Err(CoreError::NotFound);
    }
    load_page(&mut conn, page_id).await?;
    version_json(&mut conn, page_id, version_no, true).await
}
