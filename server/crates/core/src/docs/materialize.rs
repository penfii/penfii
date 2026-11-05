//! The only writer of `doc_pages.content` after creation.
//!
//! The collaboration layer hands a candidate JSON (already produced from the Yjs state) to `apply`.
//! `apply` sanitises it, drops anything the page is not entitled to reference, and when the canonical
//! form differs from what PostgreSQL holds it writes the new content, bumps `revision_no`, replaces
//! the derived reference rows and writes the event. The same state always gives the same hash, so a
//! repeat is a no-op without an event.

use std::collections::HashSet;

use serde_json::{Value, json};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use super::{content, emit_doc, replace_references};
use crate::error::CoreError;
use crate::events;
use crate::knowledge::KnowledgeConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Applied {
    /// The page is gone (deleted) or never existed; nothing was written.
    Skipped,
    /// The room's lineage was replaced (a Save or an operator reset) since it was opened; its state is
    /// no longer the page's and nothing was written.
    Stale,
    /// The canonical content equals what PostgreSQL holds.
    Unchanged {
        revision_no: i64,
    },
    Written {
        revision_no: i64,
    },
}

/// Why a candidate could not be stored. The caller records `invalid_since` and keeps the last good
/// content; it must not retry the same state in a loop.
#[derive(Debug)]
pub enum Refused {
    Invalid(String),
    Db(CoreError),
}

impl From<sqlx::Error> for Refused {
    fn from(error: sqlx::Error) -> Self {
        Refused::Db(error.into())
    }
}

impl From<CoreError> for Refused {
    fn from(error: CoreError) -> Self {
        Refused::Db(error)
    }
}

pub async fn apply(
    pool: &PgPool,
    config: &KnowledgeConfig,
    page_id: Uuid,
    candidate: &Value,
    editor_ids: &[Uuid],
    epoch: Option<Uuid>,
) -> Result<Applied, Refused> {
    let max = usize::try_from(config.doc_max_content_bytes).unwrap_or(usize::MAX);
    let mut tx = pool.begin().await?;
    let Some(page) = sqlx::query(
        "SELECT workspace_id, content_hash, revision_no FROM penfii.doc_pages
         WHERE id=$1 AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(page_id)
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(Applied::Skipped);
    };
    // A room whose lineage was replaced under it (a Save reseeds it) must not write its old state over
    // what was saved: the check is made under the page lock, the same lock Save takes.
    if let Some(epoch) = epoch {
        let current: Option<Uuid> =
            sqlx::query_scalar("SELECT epoch FROM penfii.doc_collab_state WHERE page_id=$1")
                .bind(page_id)
                .fetch_optional(&mut *tx)
                .await?;
        if current != Some(epoch) {
            return Ok(Applied::Stale);
        }
    }
    let workspace: Uuid = page.try_get("workspace_id")?;
    let old_hash: String = page.try_get("content_hash")?;
    let revision: i64 = page.try_get("revision_no")?;

    // A file may be referenced only if it was attached to this page on purpose (upload + attach).
    // Otherwise a collaborator could name any file id and hand its readers to the page's readers.
    let attached: HashSet<Uuid> = sqlx::query_scalar(
        "SELECT file_id FROM penfii.doc_page_files WHERE page_id=$1
             UNION SELECT file_id FROM penfii.doc_version_files WHERE page_id=$1",
    )
    .bind(page_id)
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .collect();
    let (_, first) = content::sanitize(candidate, max).map_err(|e| Refused::Invalid(e.0))?;
    let filtered;
    let candidate = if first.files.iter().all(|f| attached.contains(f)) {
        candidate
    } else {
        filtered = content::filter_files(candidate, &|f| attached.contains(&f));
        &filtered
    };
    let (clean, found) = content::sanitize(candidate, max).map_err(|e| Refused::Invalid(e.0))?;
    let hash = content::hash(&clean);
    if hash == old_hash {
        return Ok(Applied::Unchanged {
            revision_no: revision,
        });
    }
    let editors: Vec<Uuid> = editor_ids.to_vec();
    let revision: i64 = sqlx::query_scalar(
        "UPDATE penfii.doc_pages
         SET content=$2, content_hash=$3, revision_no=revision_no+1, materialized_at=now(), updated_at=now(), updated_by=$4
         WHERE id=$1 RETURNING revision_no",
    )
    .bind(page_id)
    .bind(&clean)
    .bind(&hash)
    .bind(editors.first().copied())
    .fetch_one(&mut *tx)
    .await?;
    finish_write(
        &mut tx,
        config,
        page_id,
        workspace,
        revision,
        &found,
        editors.first().copied(),
        json!({"source": "materialize", "editorIds": editors}),
    )
    .await?;
    tx.commit().await?;
    Ok(Applied::Written {
        revision_no: revision,
    })
}

/// What follows every write of `doc_pages.content`, whoever wrote it (the materializer or a Save):
/// the derived reference rows, the file attachments that no node names any more, and the events.
#[allow(clippy::too_many_arguments)]
pub(super) async fn finish_write(
    tx: &mut sqlx::PgConnection,
    config: &KnowledgeConfig,
    page_id: Uuid,
    workspace: Uuid,
    revision: i64,
    found: &content::Extracted,
    actor: Option<Uuid>,
    facts: Value,
) -> Result<(), CoreError> {
    let targets_changed = replace_references(tx, page_id, found, None).await?;
    // Files no node refers to any more are detached after a grace period (T43); a node that
    // appears again within the period keeps the file.
    let referenced: Vec<Uuid> = found.files.iter().copied().collect();
    sqlx::query(
        "UPDATE penfii.doc_page_files SET unreferenced_since = now()
         WHERE page_id=$1 AND unreferenced_since IS NULL AND NOT (file_id = ANY($2))
           AND attached_at < now() - make_interval(secs => $3)",
    )
    .bind(page_id)
    .bind(&referenced)
    .bind(config.doc_file_orphan_grace_seconds as f64)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "DELETE FROM penfii.doc_page_files
         WHERE page_id=$1 AND unreferenced_since IS NOT NULL
           AND unreferenced_since < now() - make_interval(secs => $2)
           AND file_id NOT IN (SELECT file_id FROM penfii.doc_version_files WHERE page_id=$1)",
    )
    .bind(page_id)
    .bind(config.doc_file_orphan_grace_seconds as f64)
    .execute(&mut *tx)
    .await?;
    let mut event = emit_doc(
        workspace,
        page_id,
        Some(revision),
        "doc.updated",
        actor.unwrap_or_else(Uuid::nil),
        facts,
    );
    if actor.is_none() {
        event.actor_user_id = None;
    }
    events::emit(&mut *tx, event).await?;
    if targets_changed {
        let mut event = emit_doc(
            workspace,
            page_id,
            Some(revision),
            "doc.targets_changed",
            Uuid::nil(),
            json!({}),
        );
        event.actor_user_id = actor;
        events::emit(&mut *tx, event).await?;
    }
    Ok(())
}
