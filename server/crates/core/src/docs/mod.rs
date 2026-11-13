//! Docs: pages in a tree, participants, templates, comments, references and attachments.
//!
//! PostgreSQL holds every fact here. `doc_pages.content` is written only by page creation and by the
//! materializer (`materialize.rs`) and by Save (`history.rs`), which compares `revision_no`. Versions
//! (`doc_versions`) are written only by Create Version.
//! Commands begin and commit their own transaction, write their events in it, and take a row lock
//! on the page first so that, per page, event order equals commit order (section E).

pub mod collab;
pub mod content;
pub mod history;
pub mod materialize;
pub mod yjs;

use std::collections::{BTreeSet, HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool, Row};
use uuid::Uuid;

use crate::access;
use crate::conversation::richtext;
use crate::error::CoreError;
use crate::events::{self, NewEvent};
use crate::knowledge::KnowledgeConfig;
use crate::visibility::{self, DocRights};

const MAX_TITLE_CHARS: usize = 200;
const MAX_TEMPLATE_NAME_CHARS: usize = 100;
const MAX_PARTICIPANTS: usize = 500;

pub struct NewPage {
    pub title: String,
    pub parent_id: Option<Uuid>,
    pub project_id: Option<Uuid>,
    pub visibility: String,
    /// With visibility PROJECT: the Project whose people may read it, and optionally a Team that narrows them.
    pub scope_project_id: Option<Uuid>,
    pub scope_team_id: Option<Uuid>,
    pub participants: Vec<(Uuid, String)>,
    pub template_id: Option<Uuid>,
    pub content: Option<Value>,
}

fn time(value: DateTime<Utc>) -> Value {
    json!(value.to_rfc3339())
}

fn clean_title(title: &str) -> Result<String, CoreError> {
    let title = title.trim();
    if title.is_empty() {
        return Err(CoreError::validation("title is required"));
    }
    if title.chars().count() > MAX_TITLE_CHARS {
        return Err(CoreError::validation("title is too long"));
    }
    Ok(title.to_owned())
}

fn check_visibility(value: &str) -> Result<(), CoreError> {
    match value {
        "WORKSPACE" | "RESTRICTED" | "PROJECT" => Ok(()),
        _ => Err(CoreError::validation(
            "visibility is WORKSPACE, PROJECT or RESTRICTED",
        )),
    }
}

async fn rights_of(
    conn: &mut PgConnection,
    actor: Uuid,
    page_id: Uuid,
) -> Result<DocRights, CoreError> {
    Ok(visibility::doc_rights(conn, actor, &[page_id])
        .await?
        .get(&page_id)
        .copied()
        .unwrap_or_default())
}

/// Locks the page row and returns its Workspace. A page the person cannot read is not found.
async fn lock_readable(
    conn: &mut PgConnection,
    actor: Uuid,
    page_id: Uuid,
) -> Result<(Uuid, DocRights), CoreError> {
    let workspace: Uuid = sqlx::query_scalar(
        "SELECT workspace_id FROM penfii.doc_pages WHERE id=$1 AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(page_id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(CoreError::NotFound)?;
    let rights = rights_of(conn, actor, page_id).await?;
    if !rights.read {
        return Err(CoreError::NotFound);
    }
    Ok((workspace, rights))
}

async fn active_members(
    conn: &mut PgConnection,
    workspace_id: Uuid,
    users: &[Uuid],
) -> Result<HashSet<Uuid>, CoreError> {
    if users.is_empty() {
        return Ok(HashSet::new());
    }
    let rows: Vec<Uuid> = sqlx::query_scalar(
        "SELECT user_id FROM penfii.workspace_members WHERE workspace_id=$1 AND user_id = ANY($2) AND state='ACTIVE'",
    )
    .bind(workspace_id)
    .bind(users)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().collect())
}

/// The stronger of two roles given to one person wins.
fn participant_rank(role: &str) -> u8 {
    match role {
        "MANAGER" => 2,
        "EDITOR" => 1,
        _ => 0,
    }
}

/// Validates a full participant set: known roles, active Workspace members, no repeats.
async fn clean_participants(
    conn: &mut PgConnection,
    workspace_id: Uuid,
    participants: &[(Uuid, String)],
) -> Result<Vec<(Uuid, String)>, CoreError> {
    if participants.len() > MAX_PARTICIPANTS {
        return Err(CoreError::validation("too many participants"));
    }
    let mut merged: Vec<(Uuid, String)> = Vec::new();
    for (user, role) in participants {
        if !matches!(role.as_str(), "MANAGER" | "EDITOR" | "VIEWER") {
            return Err(CoreError::validation(
                "participant role is MANAGER, EDITOR or VIEWER",
            ));
        }
        match merged.iter_mut().find(|(u, _)| u == user) {
            Some(existing) => {
                if participant_rank(role) > participant_rank(&existing.1) {
                    existing.1 = role.clone();
                }
            }
            None => merged.push((*user, role.clone())),
        }
    }
    let ids: Vec<Uuid> = merged.iter().map(|(u, _)| *u).collect();
    let members = active_members(conn, workspace_id, &ids).await?;
    if ids.iter().any(|u| !members.contains(u)) {
        return Err(CoreError::validation(
            "participants must be active members of the Workspace",
        ));
    }
    Ok(merged)
}

fn emit_doc(
    workspace_id: Uuid,
    page_id: Uuid,
    version: Option<i64>,
    kind: &'static str,
    actor: Uuid,
    facts: Value,
) -> NewEvent<'static> {
    NewEvent {
        workspace_id,
        target_kind: "DOC",
        target_id: page_id,
        target_version: version,
        kind,
        actor_user_id: Some(actor),
        command_id: Uuid::new_v4(),
        facts,
        // Access to a Doc is decided from PostgreSQL when each consumer acts; the event says nothing
        // about who may see it.
        security_context: json!({}),
    }
}

// ---------------------------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------------------------

struct PageRow {
    id: Uuid,
    workspace_id: Uuid,
    project_id: Option<Uuid>,
    parent_id: Option<Uuid>,
    title: String,
    visibility: String,
    scope_project_id: Option<Uuid>,
    scope_team_id: Option<Uuid>,
    position: i32,
    content: Value,
    revision_no: i64,
    materialized_at: DateTime<Utc>,
    updated_by: Option<Uuid>,
    updated_at: DateTime<Utc>,
    created_by: Uuid,
}

async fn load_page(conn: &mut PgConnection, page_id: Uuid) -> Result<PageRow, CoreError> {
    let row = sqlx::query(
        "SELECT id,workspace_id,project_id,parent_id,title,visibility,scope_project_id,scope_team_id,position,content,revision_no,
                materialized_at,updated_by,updated_at,created_by
         FROM penfii.doc_pages WHERE id=$1 AND deleted_at IS NULL",
    )
    .bind(page_id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(CoreError::NotFound)?;
    Ok(PageRow {
        id: row.try_get("id")?,
        workspace_id: row.try_get("workspace_id")?,
        project_id: row.try_get("project_id")?,
        parent_id: row.try_get("parent_id")?,
        title: row.try_get("title")?,
        visibility: row.try_get("visibility")?,
        scope_project_id: row.try_get("scope_project_id")?,
        scope_team_id: row.try_get("scope_team_id")?,
        position: row.try_get("position")?,
        content: row.try_get("content")?,
        revision_no: row.try_get("revision_no")?,
        materialized_at: row.try_get("materialized_at")?,
        updated_by: row.try_get("updated_by")?,
        updated_at: row.try_get("updated_at")?,
        created_by: row.try_get("created_by")?,
    })
}

async fn page_json(
    conn: &mut PgConnection,
    actor: Uuid,
    page: PageRow,
    live_editing: bool,
) -> Result<Value, CoreError> {
    let rights = rights_of(conn, actor, page.id).await?;
    let role: Option<String> = sqlx::query_scalar(
        "SELECT access_role FROM penfii.doc_page_participants WHERE page_id=$1 AND user_id=$2",
    )
    .bind(page.id)
    .bind(actor)
    .fetch_optional(&mut *conn)
    .await?;
    // The latest Version and whether the saved page has moved on from it.
    let latest = sqlx::query(
        "SELECT v.version_no, (v.content_hash = p.content_hash AND v.title = p.title) AS same
         FROM penfii.doc_versions v JOIN penfii.doc_pages p ON p.id = v.page_id
         WHERE v.page_id=$1 ORDER BY v.version_no DESC LIMIT 1",
    )
    .bind(page.id)
    .fetch_optional(&mut *conn)
    .await?;
    let latest_version_no = latest
        .as_ref()
        .map(|r| r.try_get::<i32, _>("version_no"))
        .transpose()?;
    let has_unversioned_changes = match &latest {
        Some(r) => !r.try_get::<bool, _>("same")?,
        None => true,
    };
    Ok(json!({
        "id": page.id,
        "latestVersionNo": latest_version_no,
        "hasUnversionedChanges": has_unversioned_changes,
        "workspaceId": page.workspace_id,
        "projectId": page.project_id,
        "parentId": page.parent_id,
        "title": page.title,
        "visibility": page.visibility,
        "scopeProjectId": page.scope_project_id,
        "scopeTeamId": page.scope_team_id,
        "position": page.position,
        "content": page.content,
        "revisionNo": page.revision_no,
        "materializedAt": time(page.materialized_at),
        "updatedAt": time(page.updated_at),
        "updatedBy": page.updated_by,
        "createdBy": page.created_by,
        "liveEditing": live_editing,
        "myRole": role,
        "rights": {"edit": rights.edit, "manage": rights.manage},
    }))
}

/// `live_editing` says a co-editing room is open for the page; the caller (HTTP) knows that, Docs do not.
pub async fn get(
    pool: &PgPool,
    actor: Uuid,
    page_id: Uuid,
    live_editing: bool,
) -> Result<Value, CoreError> {
    let mut conn = pool.acquire().await?;
    let rights = rights_of(&mut conn, actor, page_id).await?;
    if !rights.read {
        return Err(CoreError::NotFound);
    }
    let page = load_page(&mut conn, page_id).await?;
    page_json(&mut conn, actor, page, live_editing).await
}

pub async fn tree(
    pool: &PgPool,
    actor: Uuid,
    workspace_id: Uuid,
    project_id: Option<Uuid>,
) -> Result<Value, CoreError> {
    let mut conn = pool.acquire().await?;
    access::require_member(&mut conn, workspace_id, actor).await?;
    if let Some(project) = project_id {
        access::require_project_participant(&mut conn, project, actor).await?;
    }
    let rows = sqlx::query(
        "SELECT id,parent_id,title,position,visibility,project_id,updated_at
         FROM penfii.doc_pages
         WHERE workspace_id=$1 AND deleted_at IS NULL AND ($2::uuid IS NULL OR project_id=$2)
         ORDER BY position, created_at, id",
    )
    .bind(workspace_id)
    .bind(project_id)
    .fetch_all(&mut *conn)
    .await?;
    let ids: Vec<Uuid> = rows
        .iter()
        .map(|r| r.try_get("id"))
        .collect::<Result<_, _>>()?;
    let rights = visibility::doc_rights(&mut conn, actor, &ids).await?;
    let readable = |id: &Uuid| rights.get(id).is_some_and(|r| r.read);
    let mut nodes = Vec::new();
    for row in rows {
        let id: Uuid = row.try_get("id")?;
        if !readable(&id) {
            continue;
        }
        let parent: Option<Uuid> = row.try_get("parent_id")?;
        // A readable page under a page the person cannot read is shown at the top, with no hint.
        let parent = parent.filter(readable);
        let mine = rights[&id];
        nodes.push(json!({
            "id": id,
            "parentId": parent,
            "title": row.try_get::<String, _>("title")?,
            "position": row.try_get::<i32, _>("position")?,
            "visibility": row.try_get::<String, _>("visibility")?,
            "projectId": row.try_get::<Option<Uuid>, _>("project_id")?,
            "updatedAt": time(row.try_get("updated_at")?),
            "rights": {"edit": mine.edit, "manage": mine.manage},
        }));
    }
    Ok(json!({"nodes": nodes}))
}

// ---------------------------------------------------------------------------------------------
// Creating, renaming, moving
// ---------------------------------------------------------------------------------------------

async fn next_position(
    conn: &mut PgConnection,
    workspace_id: Uuid,
    parent_id: Option<Uuid>,
) -> Result<i32, CoreError> {
    Ok(sqlx::query_scalar(
        "SELECT COALESCE(max(position),0)+1 FROM penfii.doc_pages
         WHERE workspace_id=$1 AND parent_id IS NOT DISTINCT FROM $2 AND deleted_at IS NULL",
    )
    .bind(workspace_id)
    .bind(parent_id)
    .fetch_one(&mut *conn)
    .await?)
}

async fn check_project(
    conn: &mut PgConnection,
    workspace_id: Uuid,
    project_id: Option<Uuid>,
) -> Result<(), CoreError> {
    if let Some(project) = project_id {
        let found: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM penfii.projects WHERE id=$1 AND workspace_id=$2)",
        )
        .bind(project)
        .bind(workspace_id)
        .fetch_one(&mut *conn)
        .await?;
        if !found {
            return Err(CoreError::validation("unknown Project"));
        }
    }
    Ok(())
}

pub async fn create(
    pool: &PgPool,
    config: &KnowledgeConfig,
    actor: Uuid,
    workspace_id: Uuid,
    input: NewPage,
) -> Result<Value, CoreError> {
    let title = clean_title(&input.title)?;
    check_visibility(&input.visibility)?;
    if input.template_id.is_some() && input.content.is_some() {
        return Err(CoreError::validation("use a template or content, not both"));
    }
    let mut tx = pool.begin().await?;
    access::require_member(&mut tx, workspace_id, actor).await?;
    check_project(&mut tx, workspace_id, input.project_id).await?;
    if let Some(project) = input.project_id {
        access::require_project_participant(&mut tx, project, actor).await?;
    }
    if let Some(parent) = input.parent_id {
        let parent_workspace: Option<Uuid> = sqlx::query_scalar(
            "SELECT workspace_id FROM penfii.doc_pages WHERE id=$1 AND deleted_at IS NULL FOR SHARE",
        )
        .bind(parent)
        .fetch_optional(&mut *tx)
        .await?;
        let rights = rights_of(&mut tx, actor, parent).await?;
        if parent_workspace != Some(workspace_id) || !rights.read {
            return Err(CoreError::rule(
                "doc_parent_invalid",
                "the parent page is not available",
            ));
        }
        if !rights.edit {
            return Err(CoreError::rule(
                "doc_parent_invalid",
                "you cannot add pages under that page",
            ));
        }
    }
    let raw_content = match (&input.template_id, input.content) {
        (Some(template), _) => sqlx::query_scalar::<_, Value>(
            "SELECT content FROM penfii.doc_templates WHERE id=$1 AND workspace_id=$2 AND deleted_at IS NULL",
        )
        .bind(template)
        .bind(workspace_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| CoreError::validation("unknown template"))?,
        (None, Some(content)) => content,
        (None, None) => content::empty(),
    };
    let max = usize::try_from(config.doc_max_content_bytes).unwrap_or(usize::MAX);
    let (clean, found) = content::sanitize(&raw_content, max)
        .map_err(|e| CoreError::rule("doc_content_invalid", e.0))?;
    let mut participants = clean_participants(&mut tx, workspace_id, &input.participants).await?;
    // The creator manages the page, whatever the list says.
    match participants.iter_mut().find(|(u, _)| *u == actor) {
        Some(me) => me.1 = "MANAGER".into(),
        None => participants.push((actor, "MANAGER".into())),
    }
    let scope = visibility::check_share_scope(
        &mut tx,
        actor,
        workspace_id,
        &input.visibility,
        input.scope_project_id,
        input.scope_team_id,
    )
    .await?;
    let id = Uuid::new_v4();
    let position = next_position(&mut tx, workspace_id, input.parent_id).await?;
    sqlx::query(
        "INSERT INTO penfii.doc_pages(id,workspace_id,project_id,parent_id,title,visibility,scope_project_id,scope_team_id,position,content,content_schema_version,content_hash,updated_by,created_by)
         VALUES($1,$2,$3,$4,$5,$6,$11,$12,$7,$8,1,$9,$10,$10)",
    )
    .bind(id)
    .bind(workspace_id)
    .bind(input.project_id)
    .bind(input.parent_id)
    .bind(&title)
    .bind(&input.visibility)
    .bind(position)
    .bind(&clean)
    .bind(content::hash(&clean))
    .bind(actor)
    .bind(scope.0)
    .bind(scope.1)
    .execute(&mut *tx)
    .await?;
    for (user, role) in &participants {
        sqlx::query("INSERT INTO penfii.doc_page_participants(page_id,user_id,access_role) VALUES($1,$2,$3)")
            .bind(id)
            .bind(user)
            .bind(role)
            .execute(&mut *tx)
            .await?;
    }
    // Files named by explicit content must be ones the author may already read.
    if !found.files.is_empty() {
        let wanted: Vec<Uuid> = found.files.iter().copied().collect();
        let readable = visibility::files(&mut tx, actor, &wanted).await?;
        if wanted.iter().any(|f| !readable.contains(f)) {
            return Err(CoreError::rule(
                "doc_content_invalid",
                "the content names a file you cannot use",
            ));
        }
    }
    replace_references(&mut tx, id, &found, None).await?;
    events::emit(
        &mut tx,
        emit_doc(
            workspace_id,
            id,
            Some(1),
            "doc.created",
            actor,
            json!({"parentId": input.parent_id, "templateId": input.template_id}),
        ),
    )
    .await?;
    let page = load_page(&mut tx, id).await?;
    let value = page_json(&mut tx, actor, page, false).await?;
    tx.commit().await?;
    Ok(value)
}

/// Replaces the derived reference rows and the file attachments of a page from freshly extracted
/// content. Returns whether the reference rows differ from before.
pub(crate) async fn replace_references(
    conn: &mut PgConnection,
    page_id: Uuid,
    found: &content::Extracted,
    attached_by: Option<Uuid>,
) -> Result<bool, CoreError> {
    let before: BTreeSet<(String, Uuid)> =
        sqlx::query("SELECT target_kind,target_id FROM penfii.doc_page_targets WHERE page_id=$1")
            .bind(page_id)
            .fetch_all(&mut *conn)
            .await?
            .into_iter()
            .map(|r| Ok((r.try_get("target_kind")?, r.try_get("target_id")?)))
            .collect::<Result<_, sqlx::Error>>()?;
    let changed = before != found.targets;
    if changed {
        sqlx::query("DELETE FROM penfii.doc_page_targets WHERE page_id=$1")
            .bind(page_id)
            .execute(&mut *conn)
            .await?;
        for (kind, id) in &found.targets {
            sqlx::query("INSERT INTO penfii.doc_page_targets(page_id,target_kind,target_id) VALUES($1,$2,$3) ON CONFLICT DO NOTHING")
                .bind(page_id)
                .bind(kind)
                .bind(id)
                .execute(&mut *conn)
                .await?;
        }
    }
    let files: Vec<Uuid> = found.files.iter().copied().collect();
    for file in &files {
        sqlx::query(
            "INSERT INTO penfii.doc_page_files(page_id,file_id,attached_by) VALUES($1,$2,$3)
             ON CONFLICT (page_id,file_id) DO UPDATE SET unreferenced_since = NULL",
        )
        .bind(page_id)
        .bind(file)
        .bind(attached_by)
        .execute(&mut *conn)
        .await?;
    }
    Ok(changed)
}

pub async fn patch(
    pool: &PgPool,
    actor: Uuid,
    page_id: Uuid,
    title: Option<String>,
    project_id: Option<Option<Uuid>>,
) -> Result<Value, CoreError> {
    let title = title.map(|t| clean_title(&t)).transpose()?;
    let mut tx = pool.begin().await?;
    let (workspace, rights) = lock_readable(&mut tx, actor, page_id).await?;
    if title.is_some() && !rights.edit {
        return Err(CoreError::forbidden("you cannot rename this page"));
    }
    if project_id.is_some() && !rights.manage {
        return Err(CoreError::forbidden(
            "you cannot move this page to a Project",
        ));
    }
    if let Some(project) = project_id {
        check_project(&mut tx, workspace, project).await?;
        if let Some(project) = project {
            access::require_project_participant(&mut tx, project, actor).await?;
        }
        sqlx::query("UPDATE penfii.doc_pages SET project_id=$2, updated_at=now() WHERE id=$1")
            .bind(page_id)
            .bind(project)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(title) = title {
        let version: i64 = sqlx::query_scalar(
            "UPDATE penfii.doc_pages SET title=$2, revision_no=revision_no+1, updated_by=$3, updated_at=now()
             WHERE id=$1 AND title IS DISTINCT FROM $2 RETURNING revision_no",
        )
        .bind(page_id)
        .bind(&title)
        .bind(actor)
        .fetch_optional(&mut *tx)
        .await?
        .unwrap_or(0);
        if version > 0 {
            events::emit(
                &mut tx,
                emit_doc(
                    workspace,
                    page_id,
                    Some(version),
                    "doc.updated",
                    actor,
                    json!({"source": "title", "editorIds": [actor]}),
                ),
            )
            .await?;
        }
    } else if project_id.is_some() {
        // A Project change moves the page between filters; the document's project field follows.
        let version: i64 =
            sqlx::query_scalar("SELECT revision_no FROM penfii.doc_pages WHERE id=$1")
                .bind(page_id)
                .fetch_one(&mut *tx)
                .await?;
        events::emit(
            &mut tx,
            emit_doc(
                workspace,
                page_id,
                Some(version),
                "doc.updated",
                actor,
                json!({"source": "project", "editorIds": [actor]}),
            ),
        )
        .await?;
    }
    let page = load_page(&mut tx, page_id).await?;
    let value = page_json(&mut tx, actor, page, false).await?;
    tx.commit().await?;
    Ok(value)
}

pub async fn move_page(
    pool: &PgPool,
    actor: Uuid,
    page_id: Uuid,
    parent_id: Option<Uuid>,
    before_id: Option<Uuid>,
) -> Result<(), CoreError> {
    let mut tx = pool.begin().await?;
    let workspace: Uuid = sqlx::query_scalar(
        "SELECT workspace_id FROM penfii.doc_pages WHERE id=$1 AND deleted_at IS NULL",
    )
    .bind(page_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(CoreError::NotFound)?;
    // One tree edit at a time per Workspace: two moves that are fine alone could close a loop.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('doc-tree:' || $1::text, 0))")
        .bind(workspace)
        .execute(&mut *tx)
        .await?;
    let (_, rights) = lock_readable(&mut tx, actor, page_id).await?;
    if !rights.edit {
        return Err(CoreError::forbidden("you cannot move this page"));
    }
    let old_parent: Option<Uuid> =
        sqlx::query_scalar("SELECT parent_id FROM penfii.doc_pages WHERE id=$1")
            .bind(page_id)
            .fetch_one(&mut *tx)
            .await?;
    if let Some(target) = parent_id {
        let target_workspace: Option<Uuid> = sqlx::query_scalar(
            "SELECT workspace_id FROM penfii.doc_pages WHERE id=$1 AND deleted_at IS NULL",
        )
        .bind(target)
        .fetch_optional(&mut *tx)
        .await?;
        let target_rights = rights_of(&mut tx, actor, target).await?;
        if target_workspace != Some(workspace) || !target_rights.read || !target_rights.edit {
            return Err(CoreError::rule(
                "doc_parent_invalid",
                "the new parent page is not available",
            ));
        }
        // The new parent must not be the page itself or any page below it.
        let loops: bool = sqlx::query_scalar(
            "WITH RECURSIVE up AS (
               SELECT id, parent_id FROM penfii.doc_pages WHERE id=$1
               UNION ALL
               SELECT p.id, p.parent_id FROM penfii.doc_pages p JOIN up ON p.id = up.parent_id
             ) SELECT EXISTS(SELECT 1 FROM up WHERE id=$2)",
        )
        .bind(target)
        .bind(page_id)
        .fetch_one(&mut *tx)
        .await?;
        if loops {
            return Err(CoreError::rule(
                "doc_cycle",
                "a page cannot be moved into itself or its own subpage",
            ));
        }
    }
    let position = match before_id {
        Some(before) => {
            let row = sqlx::query(
                "SELECT position, parent_id FROM penfii.doc_pages WHERE id=$1 AND workspace_id=$2 AND deleted_at IS NULL",
            )
            .bind(before)
            .bind(workspace)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| CoreError::validation("beforeId is not a page"))?;
            let sibling_parent: Option<Uuid> = row.try_get("parent_id")?;
            if sibling_parent != parent_id {
                return Err(CoreError::validation(
                    "beforeId is not under the new parent",
                ));
            }
            let position: i32 = row.try_get("position")?;
            sqlx::query(
                "UPDATE penfii.doc_pages SET position=position+1
                 WHERE workspace_id=$1 AND parent_id IS NOT DISTINCT FROM $2 AND deleted_at IS NULL
                   AND position >= $3 AND id <> $4",
            )
            .bind(workspace)
            .bind(parent_id)
            .bind(position)
            .bind(page_id)
            .execute(&mut *tx)
            .await?;
            position
        }
        None => next_position(&mut tx, workspace, parent_id).await?,
    };
    sqlx::query(
        "UPDATE penfii.doc_pages SET parent_id=$2, position=$3, updated_at=now() WHERE id=$1",
    )
    .bind(page_id)
    .bind(parent_id)
    .bind(position)
    .execute(&mut *tx)
    .await?;
    events::emit(
        &mut tx,
        emit_doc(
            workspace,
            page_id,
            None,
            "doc.moved",
            actor,
            json!({"oldParentId": old_parent, "newParentId": parent_id}),
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Visibility, participants, deletion
// ---------------------------------------------------------------------------------------------

async fn manager_count(conn: &mut PgConnection, page_id: Uuid) -> Result<i64, CoreError> {
    Ok(sqlx::query_scalar(
        "SELECT count(*) FROM penfii.doc_page_participants WHERE page_id=$1 AND access_role='MANAGER'",
    )
    .bind(page_id)
    .fetch_one(&mut *conn)
    .await?)
}

pub async fn set_visibility(
    pool: &PgPool,
    actor: Uuid,
    page_id: Uuid,
    visibility: &str,
    scope_project: Option<Uuid>,
    scope_team: Option<Uuid>,
) -> Result<Value, CoreError> {
    check_visibility(visibility)?;
    let mut tx = pool.begin().await?;
    let (workspace, rights) = lock_readable(&mut tx, actor, page_id).await?;
    if !rights.manage {
        return Err(CoreError::forbidden(
            "you cannot change who can see this page",
        ));
    }
    let scope = visibility::check_share_scope(
        &mut tx,
        actor,
        workspace,
        visibility,
        scope_project,
        scope_team,
    )
    .await?;
    let current = sqlx::query(
        "SELECT visibility, scope_project_id, scope_team_id FROM penfii.doc_pages WHERE id=$1",
    )
    .bind(page_id)
    .fetch_one(&mut *tx)
    .await?;
    let from: String = current.try_get("visibility")?;
    let from_scope: (Option<Uuid>, Option<Uuid>) = (
        current.try_get("scope_project_id")?,
        current.try_get("scope_team_id")?,
    );
    if from != visibility || from_scope != scope {
        if visibility != "WORKSPACE" && manager_count(&mut tx, page_id).await? == 0 {
            return Err(CoreError::rule(
                "doc_manager_required",
                "a page shared with a Project or with invited people needs at least one manager participant",
            ));
        }
        sqlx::query("UPDATE penfii.doc_pages SET visibility=$2, scope_project_id=$3, scope_team_id=$4, updated_at=now() WHERE id=$1")
            .bind(page_id)
            .bind(visibility)
            .bind(scope.0)
            .bind(scope.1)
            .execute(&mut *tx)
            .await?;
        events::emit(
            &mut tx,
            emit_doc(
                workspace,
                page_id,
                None,
                "doc.visibility_changed",
                actor,
                json!({"from": from, "to": visibility, "scopeProjectId": scope.0, "scopeTeamId": scope.1}),
            ),
        )
        .await?;
    }
    // The person may have just closed the page to themselves; answer from the new state.
    let page = load_page(&mut tx, page_id).await?;
    let value = page_json(&mut tx, actor, page, false).await?;
    tx.commit().await?;
    Ok(value)
}

pub async fn participants(pool: &PgPool, actor: Uuid, page_id: Uuid) -> Result<Value, CoreError> {
    let mut conn = pool.acquire().await?;
    let rights = rights_of(&mut conn, actor, page_id).await?;
    if !rights.read {
        return Err(CoreError::NotFound);
    }
    list_participants(&mut conn, page_id).await
}

async fn list_participants(conn: &mut PgConnection, page_id: Uuid) -> Result<Value, CoreError> {
    let rows = sqlx::query(
        "SELECT pp.user_id, u.display_name, pp.access_role
         FROM penfii.doc_page_participants pp JOIN penfii.users u ON u.id = pp.user_id
         WHERE pp.page_id=$1 ORDER BY (pp.access_role='MANAGER') DESC, u.display_name, pp.user_id",
    )
    .bind(page_id)
    .fetch_all(&mut *conn)
    .await?;
    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            Ok(json!({
                "userId": r.try_get::<Uuid, _>("user_id")?,
                "name": r.try_get::<String, _>("display_name")?,
                "role": r.try_get::<String, _>("access_role")?,
            }))
        })
        .collect::<Result<_, sqlx::Error>>()?;
    Ok(Value::Array(items))
}

pub async fn set_participants(
    pool: &PgPool,
    actor: Uuid,
    page_id: Uuid,
    participants: Vec<(Uuid, String)>,
) -> Result<Value, CoreError> {
    let mut tx = pool.begin().await?;
    let (workspace, rights) = lock_readable(&mut tx, actor, page_id).await?;
    if !rights.manage {
        return Err(CoreError::forbidden("you cannot change the participants"));
    }
    let wanted = clean_participants(&mut tx, workspace, &participants).await?;
    let visibility: String =
        sqlx::query_scalar("SELECT visibility FROM penfii.doc_pages WHERE id=$1")
            .bind(page_id)
            .fetch_one(&mut *tx)
            .await?;
    if visibility != "WORKSPACE" && !wanted.iter().any(|(_, role)| role == "MANAGER") {
        return Err(CoreError::rule(
            "doc_manager_required",
            "a page shared with a Project or with invited people needs at least one manager participant",
        ));
    }
    let before: HashMap<Uuid, String> = sqlx::query(
        "SELECT user_id, access_role FROM penfii.doc_page_participants WHERE page_id=$1",
    )
    .bind(page_id)
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .map(|r| Ok((r.try_get("user_id")?, r.try_get("access_role")?)))
    .collect::<Result<_, sqlx::Error>>()?;
    let after: HashMap<Uuid, String> = wanted.iter().cloned().collect();
    if before != after {
        sqlx::query("DELETE FROM penfii.doc_page_participants WHERE page_id=$1")
            .bind(page_id)
            .execute(&mut *tx)
            .await?;
        for (user, role) in &wanted {
            sqlx::query("INSERT INTO penfii.doc_page_participants(page_id,user_id,access_role) VALUES($1,$2,$3)")
                .bind(page_id)
                .bind(user)
                .bind(role)
                .execute(&mut *tx)
                .await?;
        }
        let added: Vec<Uuid> = after
            .keys()
            .filter(|u| !before.contains_key(u))
            .copied()
            .collect();
        let removed: Vec<Uuid> = before
            .keys()
            .filter(|u| !after.contains_key(u))
            .copied()
            .collect();
        events::emit(
            &mut tx,
            emit_doc(
                workspace,
                page_id,
                None,
                "doc.participants_changed",
                actor,
                json!({"added": added, "removed": removed}),
            ),
        )
        .await?;
    }
    let value = list_participants(&mut tx, page_id).await?;
    tx.commit().await?;
    Ok(value)
}

/// Soft-deletes a page and the pages below it that the person may manage, in one transaction.
/// A page below it that the person may not manage (a restricted page they were never given) is
/// not deleted and not revealed: it moves up to the deleted page's parent. Returns every page
/// that was deleted so rooms can be closed.
pub async fn delete(pool: &PgPool, actor: Uuid, page_id: Uuid) -> Result<Vec<Uuid>, CoreError> {
    let mut tx = pool.begin().await?;
    let workspace: Uuid = sqlx::query_scalar(
        "SELECT workspace_id FROM penfii.doc_pages WHERE id=$1 AND deleted_at IS NULL",
    )
    .bind(page_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(CoreError::NotFound)?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('doc-tree:' || $1::text, 0))")
        .bind(workspace)
        .execute(&mut *tx)
        .await?;
    let (_, rights) = lock_readable(&mut tx, actor, page_id).await?;
    if !rights.manage {
        return Err(CoreError::forbidden("you cannot delete this page"));
    }
    let parent: Option<Uuid> =
        sqlx::query_scalar("SELECT parent_id FROM penfii.doc_pages WHERE id=$1")
            .bind(page_id)
            .fetch_one(&mut *tx)
            .await?;
    let mut tree: Vec<(Uuid, Option<Uuid>)> = sqlx::query(
        "WITH RECURSIVE sub AS (
           SELECT id, parent_id FROM penfii.doc_pages WHERE id=$1
           UNION ALL
           SELECT p.id, p.parent_id FROM penfii.doc_pages p JOIN sub ON p.parent_id = sub.id
           WHERE p.deleted_at IS NULL
         ) SELECT id, parent_id FROM sub",
    )
    .bind(page_id)
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .map(|r| Ok((r.try_get("id")?, r.try_get("parent_id")?)))
    .collect::<Result<_, sqlx::Error>>()?;
    // Lock the whole subtree so a concurrent materialization or edit sees it settled.
    let ids: Vec<Uuid> = tree.iter().map(|(id, _)| *id).collect();
    sqlx::query("SELECT id FROM penfii.doc_pages WHERE id = ANY($1) ORDER BY id FOR UPDATE")
        .bind(&ids)
        .fetch_all(&mut *tx)
        .await?;
    let rights_all = visibility::doc_rights(&mut tx, actor, &ids).await?;
    // Keep what the person may not manage, together with what hangs below it.
    let mut kept: HashSet<Uuid> = HashSet::new();
    loop {
        let before = kept.len();
        for (id, parent_of) in &tree {
            if *id == page_id {
                continue;
            }
            let manageable = rights_all.get(id).is_some_and(|r| r.manage);
            let under_kept = parent_of.is_some_and(|p| kept.contains(&p));
            if !manageable || under_kept {
                kept.insert(*id);
            }
        }
        if kept.len() == before {
            break;
        }
    }
    tree.sort_by_key(|(id, _)| *id);
    let mut deleted = Vec::new();
    for (id, _) in &tree {
        if kept.contains(id) {
            continue;
        }
        deleted.push(*id);
    }
    // Pages kept whose parent is deleted move up to the deleted page's parent.
    for (id, parent_of) in &tree {
        if kept.contains(id)
            && let Some(p) = parent_of
            && deleted.contains(p)
        {
            sqlx::query("UPDATE penfii.doc_pages SET parent_id=$2, updated_at=now() WHERE id=$1")
                .bind(id)
                .bind(parent)
                .execute(&mut *tx)
                .await?;
        }
    }
    sqlx::query(
        "UPDATE penfii.doc_pages SET deleted_at=now(), updated_at=now() WHERE id = ANY($1)",
    )
    .bind(&deleted)
    .execute(&mut *tx)
    .await?;
    for id in &deleted {
        events::emit(
            &mut tx,
            emit_doc(workspace, *id, None, "doc.deleted", actor, json!({})),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(deleted)
}

// ---------------------------------------------------------------------------------------------
// Comments
// ---------------------------------------------------------------------------------------------

const MAX_COMMENT_BYTES: usize = 65_536;

fn comment_json(row: &sqlx::postgres::PgRow) -> Result<Value, sqlx::Error> {
    Ok(json!({
        "id": row.try_get::<Uuid, _>("id")?,
        "pageId": row.try_get::<Uuid, _>("page_id")?,
        "authorId": row.try_get::<Uuid, _>("author_id")?,
        "authorName": row.try_get::<String, _>("author_name")?,
        "body": row.try_get::<Value, _>("body")?,
        "createdAt": time(row.try_get("created_at")?),
        "editedAt": row.try_get::<Option<DateTime<Utc>>, _>("edited_at")?.map(time),
    }))
}

const COMMENT_SELECT: &str = "SELECT c.id,c.page_id,c.author_id,u.display_name AS author_name,c.body,c.created_at,c.edited_at
    FROM penfii.doc_comments c JOIN penfii.users u ON u.id=c.author_id";

pub async fn list_comments(
    pool: &PgPool,
    actor: Uuid,
    page_id: Uuid,
    cursor: Option<String>,
    limit: Option<i64>,
) -> Result<Value, CoreError> {
    let limit = crate::paging::require_limit(limit, 50, 100, "limit is 1 to 100")?;
    let mut conn = pool.acquire().await?;
    if !rights_of(&mut conn, actor, page_id).await?.read {
        return Err(CoreError::NotFound);
    }
    let after: Option<(DateTime<Utc>, Uuid)> = cursor
        .as_deref()
        .map(crate::paging::decode_cursor_hex)
        .transpose()?;
    let rows = sqlx::query(&format!(
        "{COMMENT_SELECT} WHERE c.page_id=$1 AND c.deleted_at IS NULL
           AND ($2::timestamptz IS NULL OR (c.created_at, c.id) > ($2, $3))
         ORDER BY c.created_at, c.id LIMIT $4"
    ))
    .bind(page_id)
    .bind(after.map(|a| a.0))
    .bind(after.map(|a| a.1).unwrap_or(Uuid::nil()))
    .bind(limit + 1)
    .fetch_all(&mut *conn)
    .await?;
    let more = rows.len() as i64 > limit;
    let mut items = Vec::new();
    let mut last: Option<(DateTime<Utc>, Uuid)> = None;
    let ids: Vec<Uuid> = rows
        .iter()
        .take(limit as usize)
        .map(|row| row.try_get("id"))
        .collect::<Result<Vec<_>, sqlx::Error>>()?;
    let mut attachments = comment_attachments(&mut conn, &ids).await?;
    for row in rows.iter().take(limit as usize) {
        last = Some((row.try_get("created_at")?, row.try_get("id")?));
        let mut item = comment_json(row)?;
        item["attachments"] = json!(
            attachments
                .remove(&row.try_get::<Uuid, _>("id")?)
                .unwrap_or_default()
        );
        items.push(item);
    }
    let next = if more {
        last.map(|(at, id)| crate::paging::encode_cursor_hex(at, id))
    } else {
        None
    };
    Ok(json!({"items": items, "nextCursor": next}))
}

/// Validates a comment body (RichText v1) and the people it mentions.
async fn comment_mentions(
    conn: &mut PgConnection,
    workspace_id: Uuid,
    body: &Value,
    allow_blank: bool,
) -> Result<Vec<Uuid>, CoreError> {
    if body.to_string().len() > MAX_COMMENT_BYTES {
        return Err(CoreError::validation("comment is too long"));
    }
    let mentions = richtext::validate(body, allow_blank).map_err(|error| match error {
        richtext::RichTextError::Empty => CoreError::validation("comment is empty"),
        richtext::RichTextError::MentionPolicy => CoreError::validation("mention is not allowed"),
        richtext::RichTextError::Invalid(reason) => {
            CoreError::validation(format!("invalid comment body: {reason}"))
        }
    })?;
    if !allow_blank && richtext::is_blank(body) {
        return Err(CoreError::validation("comment is empty"));
    }
    let mut users: Vec<Uuid> = mentions.iter().map(|m| m.user_id).collect();
    users.sort();
    users.dedup();
    let members = active_members(conn, workspace_id, &users).await?;
    if users.iter().any(|u| !members.contains(u)) {
        return Err(CoreError::validation(
            "mentioned people must be active members of the Workspace",
        ));
    }
    Ok(users)
}

/// Comments are not a capability of their own: whoever can read the page (`read`) comments, edits their own
/// and deletes their own; a MANAGER participant also deletes others'. Editing the body is `edit`.
pub async fn create_comment(
    pool: &PgPool,
    actor: Uuid,
    page_id: Uuid,
    body: Value,
    file_ids: &[Uuid],
) -> Result<Value, CoreError> {
    let mut tx = pool.begin().await?;
    let (workspace, _) = lock_readable(&mut tx, actor, page_id).await?;
    // A comment may be only files.
    let mentioned = comment_mentions(&mut tx, workspace, &body, !file_ids.is_empty()).await?;
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO penfii.doc_comments(id,page_id,author_id,body) VALUES($1,$2,$3,$4)")
        .bind(id)
        .bind(page_id)
        .bind(actor)
        .bind(&body)
        .execute(&mut *tx)
        .await?;
    events::emit(
        &mut tx,
        emit_doc(
            workspace,
            page_id,
            None,
            "doc.comment_created",
            actor,
            json!({"commentId": id, "mentionedUserIds": mentioned}),
        ),
    )
    .await?;
    let row = sqlx::query(&format!("{COMMENT_SELECT} WHERE c.id=$1"))
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
    bind_comment_files(&mut tx, actor, workspace, id, file_ids).await?;
    let mut value = comment_json(&row)?;
    value["attachments"] = json!(
        comment_attachments(&mut tx, &[id])
            .await?
            .remove(&id)
            .unwrap_or_default()
    );
    tx.commit().await?;
    Ok(value)
}

/// Resolves a Doc comment's file for someone who can read the page. `None` when the id is not one.
pub(crate) async fn locate_comment_file(
    conn: &mut PgConnection,
    actor: Uuid,
    attachment_id: Uuid,
) -> Result<Option<crate::files::FileSource>, CoreError> {
    let Some(row) = sqlx::query(
        "SELECT c.page_id, c.deleted_at AS comment_deleted_at, f.filename, f.detected_mime, f.state,
                f.storage_key, f.deleted_at
         FROM penfii.doc_comment_files cf
         JOIN penfii.doc_comments c ON c.id = cf.comment_id
         JOIN penfii.files f ON f.id = cf.file_id
         WHERE cf.id = $1",
    )
    .bind(attachment_id)
    .fetch_optional(&mut *conn)
    .await?
    else {
        return Ok(None);
    };
    let page: Uuid = row.try_get("page_id")?;
    if !rights_of(conn, actor, page).await?.read {
        return Err(CoreError::NotFound);
    }
    if row
        .try_get::<Option<DateTime<Utc>>, _>("comment_deleted_at")?
        .is_some()
        || row
            .try_get::<Option<DateTime<Utc>>, _>("deleted_at")?
            .is_some()
        || row.try_get::<String, _>("state")? != "AVAILABLE"
    {
        return Err(CoreError::NotFound);
    }
    Ok(Some(crate::files::FileSource {
        filename: row.try_get("filename")?,
        mime: row.try_get("detected_mime")?,
        storage_key: row.try_get("storage_key")?,
    }))
}

/// Binds finished uploads to a new comment.
async fn bind_comment_files(
    conn: &mut PgConnection,
    actor: Uuid,
    workspace: Uuid,
    comment_id: Uuid,
    file_ids: &[Uuid],
) -> Result<(), CoreError> {
    let mut seen = Vec::new();
    for file_id in file_ids {
        if seen.contains(file_id) {
            continue;
        }
        seen.push(*file_id);
        crate::files::claim_unbound(conn, actor, workspace, *file_id).await?;
        sqlx::query("INSERT INTO penfii.doc_comment_files(id,comment_id,file_id) VALUES($1,$2,$3)")
            .bind(Uuid::new_v4())
            .bind(comment_id)
            .bind(file_id)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// The files of the given comments, in the order they were attached.
async fn comment_attachments(
    conn: &mut PgConnection,
    comment_ids: &[Uuid],
) -> Result<std::collections::HashMap<Uuid, Vec<penfii_contracts::MessageAttachment>>, CoreError> {
    let rows = sqlx::query(
        "SELECT cf.comment_id, cf.id, cf.file_id, f.filename, f.byte_size, f.detected_mime, f.state
         FROM penfii.doc_comment_files cf JOIN penfii.files f ON f.id = cf.file_id
         WHERE cf.comment_id = ANY($1) AND f.deleted_at IS NULL
         ORDER BY cf.attached_at, cf.id",
    )
    .bind(comment_ids)
    .fetch_all(&mut *conn)
    .await?;
    let mut grouped: std::collections::HashMap<Uuid, Vec<penfii_contracts::MessageAttachment>> =
        std::collections::HashMap::new();
    for row in rows {
        grouped.entry(row.try_get("comment_id")?).or_default().push(
            penfii_contracts::MessageAttachment {
                id: row.try_get("id")?,
                file_id: row.try_get("file_id")?,
                filename: row.try_get("filename")?,
                byte_size: row.try_get("byte_size")?,
                mime: row.try_get("detected_mime")?,
                state: row.try_get("state")?,
            },
        );
    }
    Ok(grouped)
}

pub async fn edit_comment(
    pool: &PgPool,
    actor: Uuid,
    page_id: Uuid,
    comment_id: Uuid,
    body: Value,
) -> Result<Value, CoreError> {
    let mut tx = pool.begin().await?;
    let (workspace, _) = lock_readable(&mut tx, actor, page_id).await?;
    let old = sqlx::query(
        "SELECT author_id, body FROM penfii.doc_comments WHERE id=$1 AND page_id=$2 AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(comment_id)
    .bind(page_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(CoreError::NotFound)?;
    if old.try_get::<Uuid, _>("author_id")? != actor {
        return Err(CoreError::forbidden("only the author can edit a comment"));
    }
    let mentioned = comment_mentions(&mut tx, workspace, &body, false).await?;
    let previous: HashSet<Uuid> = richtext::validate(&old.try_get::<Value, _>("body")?, true)
        .map(|m| m.into_iter().map(|m| m.user_id).collect())
        .unwrap_or_default();
    let added: Vec<Uuid> = mentioned
        .iter()
        .filter(|u| !previous.contains(u))
        .copied()
        .collect();
    sqlx::query(
        "UPDATE penfii.doc_comments SET body=$2, edited_at=now(), updated_at=now() WHERE id=$1",
    )
    .bind(comment_id)
    .bind(&body)
    .execute(&mut *tx)
    .await?;
    events::emit(
        &mut tx,
        emit_doc(
            workspace,
            page_id,
            None,
            "doc.comment_edited",
            actor,
            json!({"commentId": comment_id, "addedMentionIds": added}),
        ),
    )
    .await?;
    let row = sqlx::query(&format!("{COMMENT_SELECT} WHERE c.id=$1"))
        .bind(comment_id)
        .fetch_one(&mut *tx)
        .await?;
    let mut value = comment_json(&row)?;
    value["attachments"] = json!(
        comment_attachments(&mut tx, &[comment_id])
            .await?
            .remove(&comment_id)
            .unwrap_or_default()
    );
    tx.commit().await?;
    Ok(value)
}

pub async fn delete_comment(
    pool: &PgPool,
    store: Option<&crate::files::ObjectStore>,
    actor: Uuid,
    page_id: Uuid,
    comment_id: Uuid,
) -> Result<(), CoreError> {
    let mut tx = pool.begin().await?;
    let (workspace, _) = lock_readable(&mut tx, actor, page_id).await?;
    let author: Uuid = sqlx::query_scalar(
        "SELECT author_id FROM penfii.doc_comments WHERE id=$1 AND page_id=$2 AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(comment_id)
    .bind(page_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(CoreError::NotFound)?;
    // The author, or a MANAGER participant of the page (a Workspace admin who is not one has no say).
    let manager = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM penfii.doc_page_participants WHERE page_id=$1 AND user_id=$2 AND access_role='MANAGER')",
    )
    .bind(page_id)
    .bind(actor)
    .fetch_one(&mut *tx)
    .await?;
    if author != actor && !manager {
        return Err(CoreError::forbidden(
            "only the author or a manager can delete a comment",
        ));
    }
    sqlx::query("UPDATE penfii.doc_comments SET deleted_at=now(), updated_at=now() WHERE id=$1")
        .bind(comment_id)
        .execute(&mut *tx)
        .await?;
    // The comment's files go with it: nobody can open them any more, so they are not kept.
    let files: Vec<Uuid> = sqlx::query_scalar(
        "DELETE FROM penfii.doc_comment_files WHERE comment_id=$1 RETURNING file_id",
    )
    .bind(comment_id)
    .fetch_all(&mut *tx)
    .await?;
    let _keys = crate::files::retire(&mut tx, &files).await?;
    let _ = store;
    events::emit(
        &mut tx,
        emit_doc(
            workspace,
            page_id,
            None,
            "doc.comment_deleted",
            actor,
            json!({"commentId": comment_id}),
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Templates
// ---------------------------------------------------------------------------------------------

pub async fn list_templates(
    pool: &PgPool,
    actor: Uuid,
    workspace_id: Uuid,
) -> Result<Value, CoreError> {
    let mut conn = pool.acquire().await?;
    access::require_member(&mut conn, workspace_id, actor).await?;
    let rows = sqlx::query(
        "SELECT id, name, created_by FROM penfii.doc_templates
         WHERE workspace_id=$1 AND deleted_at IS NULL ORDER BY created_at, id",
    )
    .bind(workspace_id)
    .fetch_all(&mut *conn)
    .await?;
    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            Ok(json!({
                "id": r.try_get::<Uuid, _>("id")?,
                "name": r.try_get::<String, _>("name")?,
                "createdBy": r.try_get::<Uuid, _>("created_by")?,
            }))
        })
        .collect::<Result<_, sqlx::Error>>()?;
    Ok(Value::Array(items))
}

/// A template is a copy of the page's materialized content at this moment. Files are not copied:
/// they belong to the page they were attached to, so a template carries none.
pub async fn create_template(
    pool: &PgPool,
    config: &KnowledgeConfig,
    actor: Uuid,
    workspace_id: Uuid,
    name: &str,
    from_doc_id: Uuid,
) -> Result<Value, CoreError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_TEMPLATE_NAME_CHARS {
        return Err(CoreError::validation(
            "template name is 1 to 100 characters",
        ));
    }
    let mut tx = pool.begin().await?;
    access::require_member(&mut tx, workspace_id, actor).await?;
    let rights = rights_of(&mut tx, actor, from_doc_id).await?;
    let source = sqlx::query(
        "SELECT workspace_id, content FROM penfii.doc_pages WHERE id=$1 AND deleted_at IS NULL",
    )
    .bind(from_doc_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(source) = source else {
        return Err(CoreError::NotFound);
    };
    if !rights.read || source.try_get::<Uuid, _>("workspace_id")? != workspace_id {
        return Err(CoreError::NotFound);
    }
    let stripped = content::strip_files(&source.try_get::<Value, _>("content")?);
    let max = usize::try_from(config.doc_max_content_bytes).unwrap_or(usize::MAX);
    let (clean, _) = content::sanitize(&stripped, max)
        .map_err(|e| CoreError::rule("doc_content_invalid", e.0))?;
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO penfii.doc_templates(id,workspace_id,name,content,content_schema_version,created_by) VALUES($1,$2,$3,$4,1,$5)",
    )
    .bind(id)
    .bind(workspace_id)
    .bind(name)
    .bind(&clean)
    .bind(actor)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(json!({"id": id, "name": name, "createdBy": actor}))
}

pub async fn delete_template(
    pool: &PgPool,
    actor: Uuid,
    workspace_id: Uuid,
    template_id: Uuid,
) -> Result<(), CoreError> {
    let mut tx = pool.begin().await?;
    let role = access::require_member(&mut tx, workspace_id, actor).await?;
    let creator: Uuid = sqlx::query_scalar(
        "SELECT created_by FROM penfii.doc_templates WHERE id=$1 AND workspace_id=$2 AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(template_id)
    .bind(workspace_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(CoreError::NotFound)?;
    if creator != actor && !matches!(role.as_str(), "OWNER" | "ADMIN") {
        return Err(CoreError::forbidden(
            "only the creator or an admin can delete a template",
        ));
    }
    sqlx::query("UPDATE penfii.doc_templates SET deleted_at=now() WHERE id=$1")
        .bind(template_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Attachments
// ---------------------------------------------------------------------------------------------

/// Called by the editor right after an upload so collaborators can fetch the file before the next
/// materialization lists it (T43). The file must be the author's own finished upload in this
/// Workspace and not attached to a message.
pub async fn attach_file(
    pool: &PgPool,
    actor: Uuid,
    page_id: Uuid,
    file_id: Uuid,
) -> Result<(), CoreError> {
    let mut tx = pool.begin().await?;
    let (workspace, rights) = lock_readable(&mut tx, actor, page_id).await?;
    if !rights.edit {
        return Err(CoreError::forbidden("you cannot edit this page"));
    }
    let file = sqlx::query(
        "SELECT workspace_id, created_by FROM penfii.files WHERE id=$1 AND state='AVAILABLE' AND deleted_at IS NULL",
    )
    .bind(file_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(CoreError::NotFound)?;
    if file.try_get::<Uuid, _>("workspace_id")? != workspace
        || file.try_get::<Uuid, _>("created_by")? != actor
    {
        return Err(CoreError::NotFound);
    }
    let on_message = crate::files::bound_outside_docs(&mut tx, file_id).await?;
    if on_message {
        return Err(CoreError::NotFound);
    }
    sqlx::query(
        "INSERT INTO penfii.doc_page_files(page_id,file_id,attached_by) VALUES($1,$2,$3)
         ON CONFLICT (page_id,file_id) DO UPDATE SET unreferenced_since = NULL",
    )
    .bind(page_id)
    .bind(file_id)
    .bind(actor)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// References
// ---------------------------------------------------------------------------------------------

/// What the editor shows for `reference` and `mention` nodes. A title is returned only for a
/// target the caller may read; anything else is `available:false` with no title and no hint of why.
pub async fn resolve_references(
    pool: &PgPool,
    actor: Uuid,
    workspace_id: Uuid,
    items: Vec<(String, Uuid)>,
) -> Result<Value, CoreError> {
    if items.len() > 100 {
        return Err(CoreError::validation("at most 100 items"));
    }
    let mut conn = pool.acquire().await?;
    access::require_member(&mut conn, workspace_id, actor).await?;
    let ids_of = |kind: &str| -> Vec<Uuid> {
        items
            .iter()
            .filter(|(k, _)| k == kind)
            .map(|(_, id)| *id)
            .collect()
    };
    for (kind, _) in &items {
        if !matches!(kind.as_str(), "TICKET" | "DOC" | "RECORD" | "USER") {
            return Err(CoreError::validation("kind is TICKET, DOC, RECORD or USER"));
        }
    }
    let mut found: HashMap<(String, Uuid), Value> = HashMap::new();
    let tickets = ids_of("TICKET");
    let readable = visibility::tickets(&mut conn, actor, &tickets).await?;
    if !readable.is_empty() {
        let ids: Vec<Uuid> = readable.iter().copied().collect();
        for row in sqlx::query(
            "SELECT t.id, t.public_key, t.title, t.project_id, p.name AS project_name
             FROM penfii.tickets t JOIN penfii.projects p ON p.id=t.project_id
             WHERE t.id = ANY($1) AND t.workspace_id=$2",
        )
        .bind(&ids)
        .bind(workspace_id)
        .fetch_all(&mut *conn)
        .await?
        {
            let id: Uuid = row.try_get("id")?;
            found.insert(
                ("TICKET".into(), id),
                json!({"title": format!("{} {}", row.try_get::<String, _>("public_key")?, row.try_get::<String, _>("title")?),
                       "context": row.try_get::<String, _>("project_name")?,
                       "projectId": row.try_get::<Uuid, _>("project_id")?}),
            );
        }
    }
    let docs = ids_of("DOC");
    let readable = visibility::docs(&mut conn, actor, &docs).await?;
    if !readable.is_empty() {
        let ids: Vec<Uuid> = readable.iter().copied().collect();
        for row in sqlx::query(
            "SELECT id, title FROM penfii.doc_pages WHERE id = ANY($1) AND workspace_id=$2 AND deleted_at IS NULL",
        )
        .bind(&ids)
        .bind(workspace_id)
        .fetch_all(&mut *conn)
        .await?
        {
            let id: Uuid = row.try_get("id")?;
            found.insert(("DOC".into(), id), json!({"title": row.try_get::<String, _>("title")?}));
        }
    }
    let users = ids_of("USER");
    if !users.is_empty() {
        for row in sqlx::query(
            "SELECT u.id, u.display_name FROM penfii.users u
             JOIN penfii.workspace_members wm ON wm.user_id=u.id AND wm.workspace_id=$2 AND wm.state='ACTIVE'
             WHERE u.id = ANY($1)",
        )
        .bind(&users)
        .bind(workspace_id)
        .fetch_all(&mut *conn)
        .await?
        {
            let id: Uuid = row.try_get("id")?;
            found.insert(("USER".into(), id), json!({"title": row.try_get::<String, _>("display_name")?}));
        }
    }
    // A RECORD kind survives only so a page written before Databases were removed still resolves
    // (as unavailable) instead of failing the whole request.
    let out: Vec<Value> = items
        .into_iter()
        .map(|(kind, id)| match found.get(&(kind.clone(), id)) {
            Some(info) => {
                let mut value = json!({"kind": kind, "id": id, "available": true});
                value["title"] = info["title"].clone();
                // Only what the caller may know about a readable target: where it lives, so a chip can open it.
                for key in ["context", "projectId"] {
                    if let Some(extra) = info.get(key) {
                        value[key] = extra.clone();
                    }
                }
                value
            }
            None => json!({"kind": kind, "id": id, "available": false}),
        })
        .collect();
    Ok(json!({"items": out}))
}
