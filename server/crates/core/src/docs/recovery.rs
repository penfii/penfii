//! Doc pages that nobody manages, and the one way out of it.
//!
//! A page shared with a Project or with invited people (`PROJECT`, `RESTRICTED`) is managed only by
//! its `MANAGER` participants; Workspace owners and admins get nothing extra on it. Removing a person
//! from the Workspace revokes their participant rows (FEA-149), the last manager's included, and a
//! manager can also be disabled, so such a page can be left without anyone who can manage it. Its
//! content and history are kept. A page that is open to the whole Workspace is not listed: its
//! creator and the Workspace owners and admins manage it.
//!
//! A Workspace owner or admin sees these pages by title, creator and time only, never content
//! ([`docs_without_manager`]), and names a manager for one ([`recover_manager`]), the same way a
//! Project without an admin and a private channel without a manager are recovered. Naming a manager
//! is the only thing this allows. It gives that person the rights of a manager of the page, which
//! include reading it; naming oneself is the one way an owner or admin gets any. The audit entry
//! `doc.manager_recovered` says who named whom, whether it was themself, and whether this opened the
//! page to someone who could not read it.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::CoreError;
use crate::events;
use crate::visibility;

/// A manager who is an active member of the Workspace with an account that is not disabled.
const ACTIVE_DOC_MANAGERS: &str = "(SELECT pp.page_id, pp.user_id
        FROM penfii.doc_page_participants pp
        JOIN penfii.doc_pages p ON p.id = pp.page_id
        JOIN penfii.workspace_members wm
          ON wm.workspace_id = p.workspace_id AND wm.user_id = pp.user_id AND wm.state = 'ACTIVE'
        JOIN penfii.users u ON u.id = pp.user_id AND u.disabled_at IS NULL
        WHERE pp.access_role = 'MANAGER')";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocWithoutManager {
    pub id: Uuid,
    pub title: String,
    pub visibility: String,
    pub created_by: Uuid,
    pub created_by_name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// The live `PROJECT` and `RESTRICTED` pages of the Workspace that have no active manager, oldest
/// first. Workspace owners and admins only. Title, creator and times are all it shows.
pub async fn docs_without_manager(
    pool: &PgPool,
    actor: Uuid,
    workspace_id: Uuid,
) -> Result<Vec<DocWithoutManager>, CoreError> {
    let mut conn = pool.acquire().await?;
    crate::admin::require_manager(&mut conn, workspace_id, actor).await?;
    let rows = sqlx::query_as::<_, (Uuid, String, String, Uuid, String, DateTime<Utc>, DateTime<Utc>)>(&format!(
        "SELECT p.id, p.title, p.visibility, p.created_by, u.display_name, p.created_at, p.updated_at
         FROM penfii.doc_pages p JOIN penfii.users u ON u.id = p.created_by
         WHERE p.workspace_id = $1 AND p.deleted_at IS NULL AND p.visibility <> 'WORKSPACE'
           AND NOT EXISTS (SELECT 1 FROM {m} x WHERE x.page_id = p.id)
         ORDER BY p.created_at, p.id",
        m = ACTIVE_DOC_MANAGERS
    ))
    .bind(workspace_id)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(id, title, visibility, created_by, created_by_name, created_at, updated_at)| {
                DocWithoutManager {
                    id,
                    title,
                    visibility,
                    created_by,
                    created_by_name,
                    created_at,
                    updated_at,
                }
            },
        )
        .collect())
}

/// What a recovery did.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocManagerRecovery {
    /// The named person could not read the page before, so this gave them access to its content.
    pub new_access: bool,
}

/// A Workspace owner or admin names a manager for a `PROJECT` or `RESTRICTED` page that has no active
/// manager. The page is locked, so two recoveries at once name one; a page that has an active manager
/// is refused (`doc_has_manager`), and so is one open to the whole Workspace (`doc_not_restricted`).
/// The named person must be an active member whose account is not disabled. A person who already has
/// a participant row keeps it and becomes its manager. Audited as `doc.manager_recovered`.
pub async fn recover_manager(
    pool: &PgPool,
    actor: Uuid,
    page_id: Uuid,
    user_id: Uuid,
) -> Result<DocManagerRecovery, CoreError> {
    let mut tx = pool.begin().await?;
    let page: Option<(Uuid, String)> = sqlx::query_as(
        "SELECT workspace_id, visibility FROM penfii.doc_pages
         WHERE id = $1 AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(page_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((workspace, visibility)) = page else {
        return Err(CoreError::NotFound);
    };
    crate::admin::require_manager(&mut tx, workspace, actor).await?;
    if visibility == "WORKSPACE" {
        return Err(CoreError::conflict(
            "doc_not_restricted",
            "a page open to the whole Workspace is managed by its creator and the Workspace admins",
        ));
    }
    sqlx::query(
        "SELECT user_id FROM penfii.doc_page_participants
         WHERE page_id = $1 AND access_role = 'MANAGER' ORDER BY user_id FOR UPDATE",
    )
    .bind(page_id)
    .fetch_all(&mut *tx)
    .await?;
    let has_manager: bool = sqlx::query_scalar(&format!(
        "SELECT EXISTS (SELECT 1 FROM {m} x WHERE x.page_id = $1)",
        m = ACTIVE_DOC_MANAGERS
    ))
    .bind(page_id)
    .fetch_one(&mut *tx)
    .await?;
    if has_manager {
        return Err(CoreError::conflict(
            "doc_has_manager",
            "the page has an active manager; only a page without one is recovered this way",
        ));
    }
    let target: Option<bool> = sqlx::query_scalar(
        "SELECT u.disabled_at IS NOT NULL
         FROM penfii.workspace_members wm JOIN penfii.users u ON u.id = wm.user_id
         WHERE wm.workspace_id = $1 AND wm.user_id = $2 AND wm.state = 'ACTIVE'
         FOR SHARE OF wm",
    )
    .bind(workspace)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?;
    match target {
        None => {
            return Err(CoreError::validation(
                "the person must be an active member of the Workspace",
            ));
        }
        Some(true) => {
            return Err(CoreError::validation(
                "the person's account is disabled; enable it first",
            ));
        }
        Some(false) => {}
    }
    let could_read = visibility::doc_rights(&mut tx, user_id, &[page_id])
        .await?
        .get(&page_id)
        .is_some_and(|rights| rights.read);
    let before: Option<String> = sqlx::query_scalar(
        "SELECT access_role FROM penfii.doc_page_participants WHERE page_id = $1 AND user_id = $2",
    )
    .bind(page_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO penfii.doc_page_participants (page_id, user_id, access_role) VALUES ($1, $2, 'MANAGER')
         ON CONFLICT (page_id, user_id) DO UPDATE SET access_role = 'MANAGER'",
    )
    .bind(page_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    let new_access = !could_read;
    events::emit(
        &mut tx,
        events::NewEvent {
            workspace_id: workspace,
            target_kind: "DOC",
            target_id: page_id,
            target_version: None,
            kind: "doc.participants_changed",
            actor_user_id: Some(actor),
            command_id: Uuid::new_v4(),
            facts: serde_json::json!({"added": [user_id], "removed": []}),
            security_context: serde_json::json!({}),
        },
    )
    .await?;
    crate::admin::audit(
        &mut tx,
        workspace,
        Some(actor),
        "doc.manager_recovered",
        "DOC",
        page_id,
        serde_json::json!({
            "userId": user_id,
            "self": actor == user_id,
            "newAccess": new_access,
            "previousRole": before,
        }),
    )
    .await?;
    tx.commit().await?;
    Ok(DocManagerRecovery { new_access })
}
