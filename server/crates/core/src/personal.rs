//! Personal Work that needs no events: Favorites and Later (private to one person) and Pins
//! (shared by a Project). Each stores a reference only; the title and the right to see it are
//! resolved again on every read, so a lost permission removes the target from view at once.

use penfii_contracts::{FavoriteItem, LaterItem, PinItem, ProjectPins, SavedRef, TicketRef};
use sqlx::{PgConnection, PgPool, Row};
use uuid::Uuid;

use crate::error::CoreError;
use crate::project_role::{channel_allowed, project_role, require_project_role};

const FAVORITE_KINDS: &[&str] = &["PROJECT", "TEAM", "TICKET", "VIEW", "CHANNEL"];
const LATER_KINDS: &[&str] = &["TICKET", "MESSAGE", "FILE"];
const POSITION_STEP: i64 = 1024;

fn unavailable(kind: &str, target_id: Uuid) -> SavedRef {
    SavedRef {
        kind: kind.to_owned(),
        target_id,
        available: false,
        title: None,
        subtitle: None,
        project_id: None,
        team_id: None,
        view_target: None,
        layout: None,
        conversation_id: None,
        thread_root_id: None,
        message_id: None,
    }
}

/// Resolves a reference for `actor`. `None` means the target does not exist or the person may not
/// see it; the two look the same.
async fn resolve(
    conn: &mut PgConnection,
    actor_id: Uuid,
    workspace_id: Uuid,
    kind: &str,
    target_id: Uuid,
) -> Result<Option<SavedRef>, CoreError> {
    let mut found = unavailable(kind, target_id);
    found.available = true;
    match kind {
        "PROJECT" => {
            let row = sqlx::query(
                "SELECT name FROM penfii.projects WHERE id=$1 AND workspace_id=$2 AND archived_at IS NULL",
            )
            .bind(target_id)
            .bind(workspace_id)
            .fetch_optional(&mut *conn)
            .await?;
            let Some(row) = row else { return Ok(None) };
            if project_role(conn, target_id, actor_id).await?.1.is_none() {
                return Ok(None);
            }
            found.title = Some(row.try_get("name")?);
            found.project_id = Some(target_id);
        }
        "TEAM" => {
            let row = sqlx::query("SELECT name FROM penfii.teams WHERE id=$1 AND workspace_id=$2")
                .bind(target_id)
                .bind(workspace_id)
                .fetch_optional(&mut *conn)
                .await?;
            let Some(row) = row else { return Ok(None) };
            found.title = Some(row.try_get("name")?);
            found.team_id = Some(target_id);
        }
        "TICKET" => {
            let row = sqlx::query(
                "SELECT t.public_key,t.title,t.project_id,t.channel_id,p.name AS project_name
                 FROM penfii.tickets t JOIN penfii.projects p ON p.id=t.project_id
                 WHERE t.id=$1 AND t.workspace_id=$2
                   AND t.archived_at IS NULL AND t.trashed_at IS NULL AND t.purged_at IS NULL",
            )
            .bind(target_id)
            .bind(workspace_id)
            .fetch_optional(&mut *conn)
            .await?;
            let Some(row) = row else { return Ok(None) };
            let project_id: Uuid = row.try_get("project_id")?;
            if project_role(conn, project_id, actor_id).await?.1.is_none()
                || !channel_allowed(conn, row.try_get("channel_id")?, actor_id).await?
            {
                return Ok(None);
            }
            found.title = Some(format!(
                "{} {}",
                row.try_get::<String, _>("public_key")?,
                row.try_get::<String, _>("title")?
            ));
            found.subtitle = Some(row.try_get("project_name")?);
            found.project_id = Some(project_id);
        }
        "VIEW" => {
            let row = sqlx::query(
                "SELECT name,project_id,team_id,target_kind,scope,owner_id,layout FROM penfii.project_views
                 WHERE id=$1 AND workspace_id=$2 AND deleted_at IS NULL",
            )
            .bind(target_id)
            .bind(workspace_id)
            .fetch_optional(&mut *conn)
            .await?;
            let Some(row) = row else { return Ok(None) };
            let scope: String = row.try_get("scope")?;
            let owner: Uuid = row.try_get("owner_id")?;
            let project_id: Option<Uuid> = row.try_get("project_id")?;
            if scope == "PERSONAL" && owner != actor_id {
                return Ok(None);
            }
            if let Some(project) = project_id
                && project_role(conn, project, actor_id).await?.1.is_none()
            {
                return Ok(None);
            }
            found.title = Some(row.try_get("name")?);
            found.project_id = project_id;
            found.team_id = row.try_get("team_id")?;
            found.view_target = Some(row.try_get("target_kind")?);
            found.layout = Some(row.try_get("layout")?);
        }
        "CHANNEL" => {
            let row = sqlx::query(
                "SELECT c.name,c.project_id FROM penfii.channels c
                 JOIN penfii.conversations cv ON cv.id=c.id
                 WHERE c.id=$1 AND cv.workspace_id=$2 AND c.archived_at IS NULL",
            )
            .bind(target_id)
            .bind(workspace_id)
            .fetch_optional(&mut *conn)
            .await?;
            let Some(row) = row else { return Ok(None) };
            if crate::access::load_conversation(conn, actor_id, target_id)
                .await?
                .is_none_or(|access| !access.can_read())
            {
                return Ok(None);
            }
            found.title = Some(row.try_get("name")?);
            found.project_id = row.try_get("project_id")?;
        }
        "MESSAGE" => {
            let row = sqlx::query(
                "SELECT m.conversation_id, m.thread_root_id, m.body, u.display_name, ch.name AS channel_name, ch.project_id
                 FROM penfii.messages m JOIN penfii.conversations c ON c.id=m.conversation_id
                 JOIN penfii.users u ON u.id=m.author_id LEFT JOIN penfii.channels ch ON ch.id=c.id
                 WHERE m.id=$1 AND c.workspace_id=$2",
            )
            .bind(target_id)
            .bind(workspace_id)
            .fetch_optional(&mut *conn)
            .await?;
            let Some(row) = row else { return Ok(None) };
            if !crate::visibility::messages(conn, actor_id, &[target_id])
                .await?
                .contains(&target_id)
            {
                return Ok(None);
            }
            let text = crate::work_history::description_text(
                &row.try_get::<serde_json::Value, _>("body")?,
            );
            let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
            found.title = Some(flat.chars().take(120).collect());
            found.subtitle = Some(match row.try_get::<Option<String>, _>("channel_name")? {
                Some(channel) => {
                    format!("{} · #{channel}", row.try_get::<String, _>("display_name")?)
                }
                None => row.try_get::<String, _>("display_name")?,
            });
            found.project_id = row.try_get("project_id")?;
            found.conversation_id = Some(row.try_get("conversation_id")?);
            found.thread_root_id = row.try_get("thread_root_id")?;
            found.message_id = Some(target_id);
        }
        "FILE" => {
            let row = sqlx::query(
                "SELECT f.filename, f.byte_size, u.display_name FROM penfii.files f JOIN penfii.users u ON u.id=f.created_by
                 WHERE f.id=$1 AND f.workspace_id=$2",
            )
            .bind(target_id)
            .bind(workspace_id)
            .fetch_optional(&mut *conn)
            .await?;
            let Some(row) = row else { return Ok(None) };
            if !crate::visibility::files(conn, actor_id, &[target_id])
                .await?
                .contains(&target_id)
            {
                return Ok(None);
            }
            found.title = Some(row.try_get("filename")?);
            found.subtitle = Some(format!(
                "{} · {} KB",
                row.try_get::<String, _>("display_name")?,
                (row.try_get::<i64, _>("byte_size")? + 1023) / 1024
            ));
            // The message to open it from: the first one the person can read.
            let places = sqlx::query(
                "SELECT m.id, m.conversation_id, m.thread_root_id FROM penfii.message_attachments a
                 JOIN penfii.messages m ON m.id=a.message_id AND m.deleted_at IS NULL WHERE a.file_id=$1 ORDER BY m.created_at",
            )
            .bind(target_id)
            .fetch_all(&mut *conn)
            .await?;
            for place in places {
                let conversation: Uuid = place.try_get("conversation_id")?;
                if crate::access::load_conversation(conn, actor_id, conversation)
                    .await?
                    .is_some_and(|a| a.can_read())
                {
                    found.conversation_id = Some(conversation);
                    found.message_id = Some(place.try_get("id")?);
                    found.thread_root_id = place.try_get("thread_root_id")?;
                    break;
                }
            }
        }
        _ => return Err(CoreError::validation("unknown kind")),
    }
    Ok(Some(found))
}

// ---------------------------------------------------------------------------------------------
// Favorites
// ---------------------------------------------------------------------------------------------

pub async fn list_favorites(
    pool: &PgPool,
    actor_id: Uuid,
    workspace_id: Uuid,
) -> Result<Vec<FavoriteItem>, CoreError> {
    let mut tx = pool.begin().await?;
    crate::access::require_member(&mut tx, workspace_id, actor_id).await?;
    let rows = sqlx::query(
        "SELECT kind,target_id,position,created_at,folder_id FROM penfii.favorites
         WHERE user_id=$1 AND workspace_id=$2 ORDER BY position,created_at,target_id",
    )
    .bind(actor_id)
    .bind(workspace_id)
    .fetch_all(&mut *tx)
    .await?;
    let mut items = Vec::new();
    for row in rows {
        let kind: String = row.try_get("kind")?;
        let target_id: Uuid = row.try_get("target_id")?;
        // A shortcut to something that can no longer be opened simply is not shown.
        if let Some(target) = resolve(&mut tx, actor_id, workspace_id, &kind, target_id).await? {
            items.push(FavoriteItem {
                target,
                position: row.try_get("position")?,
                created_at: row.try_get("created_at")?,
                folder_id: row.try_get("folder_id")?,
            });
        }
    }
    tx.commit().await?;
    Ok(items)
}

pub async fn add_favorite(
    pool: &PgPool,
    actor_id: Uuid,
    workspace_id: Uuid,
    kind: &str,
    target_id: Uuid,
) -> Result<FavoriteItem, CoreError> {
    if !FAVORITE_KINDS.contains(&kind) {
        return Err(CoreError::validation("unknown favorite kind"));
    }
    let mut tx = pool.begin().await?;
    crate::access::require_member(&mut tx, workspace_id, actor_id).await?;
    let target = resolve(&mut tx, actor_id, workspace_id, kind, target_id)
        .await?
        .ok_or(CoreError::NotFound)?;
    // The same target is saved once; saving it again keeps its place.
    sqlx::query(
        "INSERT INTO penfii.favorites(user_id,workspace_id,kind,target_id,position)
         SELECT $1,$2,$3,$4,coalesce(max(position),0)+$5 FROM penfii.favorites WHERE user_id=$1 AND workspace_id=$2
         ON CONFLICT (user_id,kind,target_id) DO NOTHING",
    )
    .bind(actor_id)
    .bind(workspace_id)
    .bind(kind)
    .bind(target_id)
    .bind(POSITION_STEP)
    .execute(&mut *tx)
    .await?;
    let row = sqlx::query(
        "SELECT position,created_at FROM penfii.favorites WHERE user_id=$1 AND kind=$2 AND target_id=$3",
    )
    .bind(actor_id)
    .bind(kind)
    .bind(target_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(FavoriteItem {
        target,
        position: row.try_get("position")?,
        created_at: row.try_get("created_at")?,
        folder_id: None,
    })
}

pub async fn remove_favorite(
    pool: &PgPool,
    actor_id: Uuid,
    workspace_id: Uuid,
    kind: &str,
    target_id: Uuid,
) -> Result<(), CoreError> {
    let mut tx = pool.begin().await?;
    crate::access::require_member(&mut tx, workspace_id, actor_id).await?;
    sqlx::query("DELETE FROM penfii.favorites WHERE user_id=$1 AND workspace_id=$2 AND kind=$3 AND target_id=$4")
        .bind(actor_id)
        .bind(workspace_id)
        .bind(kind)
        .bind(target_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// Sets the order of the person's Favorites. Shortcuts not named keep their relative order after
/// the named ones; names that are not Favorites are ignored.
pub async fn order_favorites(
    pool: &PgPool,
    actor_id: Uuid,
    workspace_id: Uuid,
    order: Vec<(String, Uuid)>,
) -> Result<(), CoreError> {
    if order.len() > 500 {
        return Err(CoreError::validation("too many favorites"));
    }
    let mut tx = pool.begin().await?;
    crate::access::require_member(&mut tx, workspace_id, actor_id).await?;
    sqlx::query(
        "SELECT user_id FROM penfii.favorites WHERE user_id=$1 AND workspace_id=$2 FOR UPDATE",
    )
    .bind(actor_id)
    .bind(workspace_id)
    .fetch_all(&mut *tx)
    .await?;
    for (index, (kind, target_id)) in order.iter().enumerate() {
        sqlx::query(
            "UPDATE penfii.favorites SET position=$5 WHERE user_id=$1 AND workspace_id=$2 AND kind=$3 AND target_id=$4",
        )
        .bind(actor_id)
        .bind(workspace_id)
        .bind(kind)
        .bind(target_id)
        .bind((index as i64 + 1) * POSITION_STEP)
        .execute(&mut *tx)
        .await?;
    }
    // Unnamed ones go after, keeping their order.
    sqlx::query(
        "UPDATE penfii.favorites f SET position=$3+r.rn*$4 FROM (
            SELECT kind,target_id,row_number() OVER (ORDER BY position,created_at,target_id) AS rn
            FROM penfii.favorites WHERE user_id=$1 AND workspace_id=$2 AND position<=0) r
         WHERE f.user_id=$1 AND f.kind=r.kind AND f.target_id=r.target_id AND f.position<=0",
    )
    .bind(actor_id)
    .bind(workspace_id)
    .bind((order.len() as i64 + 1) * POSITION_STEP)
    .bind(POSITION_STEP)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Later
// ---------------------------------------------------------------------------------------------

fn note_of(note: Option<String>) -> Result<Option<String>, CoreError> {
    let note = note.map(|n| n.trim().to_owned()).filter(|n| !n.is_empty());
    if note.as_ref().is_some_and(|n| n.chars().count() > 500) {
        return Err(CoreError::validation("a note is limited to 500 characters"));
    }
    Ok(note)
}

async fn later_from(
    conn: &mut PgConnection,
    actor_id: Uuid,
    row: &sqlx::postgres::PgRow,
) -> Result<Option<LaterItem>, CoreError> {
    let kind: String = row.try_get("kind")?;
    let target_id: Uuid = row.try_get("target_id")?;
    let workspace_id: Uuid = row.try_get("workspace_id")?;
    let note: Option<String> = row.try_get("note")?;
    let target = match resolve(conn, actor_id, workspace_id, &kind, target_id).await? {
        Some(target) => target,
        // Without a note there is nothing of the person's own to show; with one, only the note.
        None if note.is_some() => unavailable(&kind, target_id),
        None => return Ok(None),
    };
    let id: Uuid = row.try_get("id")?;
    let reminder = crate::reminders::latest(conn, id).await?;
    Ok(Some(LaterItem {
        id,
        target,
        note,
        state: row.try_get("state")?,
        version: row.try_get("version")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
        reminder,
    }))
}

const LATER_COLUMNS: &str =
    "id,workspace_id,kind,target_id,note,state,version,created_at,updated_at";

pub async fn list_later(
    pool: &PgPool,
    actor_id: Uuid,
    workspace_id: Uuid,
    state: &str,
) -> Result<Vec<LaterItem>, CoreError> {
    if !matches!(state, "ACTIVE" | "DONE" | "ARCHIVED") {
        return Err(CoreError::validation(
            "state must be ACTIVE, DONE or ARCHIVED",
        ));
    }
    let mut tx = pool.begin().await?;
    crate::access::require_member(&mut tx, workspace_id, actor_id).await?;
    let rows = sqlx::query(&format!(
        "SELECT {LATER_COLUMNS} FROM penfii.later_items WHERE user_id=$1 AND workspace_id=$2 AND state=$3
         ORDER BY created_at DESC, id"
    ))
    .bind(actor_id)
    .bind(workspace_id)
    .bind(state)
    .fetch_all(&mut *tx)
    .await?;
    let mut items = Vec::new();
    for row in &rows {
        items.extend(later_from(&mut tx, actor_id, row).await?);
    }
    tx.commit().await?;
    Ok(items)
}

pub async fn add_later(
    pool: &PgPool,
    actor_id: Uuid,
    workspace_id: Uuid,
    kind: &str,
    target_id: Uuid,
    note: Option<String>,
) -> Result<LaterItem, CoreError> {
    if !LATER_KINDS.contains(&kind) {
        return Err(CoreError::validation("unknown kind for Later"));
    }
    let note = note_of(note)?;
    let mut tx = pool.begin().await?;
    crate::access::require_member(&mut tx, workspace_id, actor_id).await?;
    resolve(&mut tx, actor_id, workspace_id, kind, target_id)
        .await?
        .ok_or(CoreError::NotFound)?;
    // Saving the same object again keeps the one card.
    let inserted = sqlx::query(
        "INSERT INTO penfii.later_items(id,user_id,workspace_id,kind,target_id,note)
         VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT (user_id,kind,target_id) WHERE state='ACTIVE' DO NOTHING",
    )
    .bind(Uuid::new_v4())
    .bind(actor_id)
    .bind(workspace_id)
    .bind(kind)
    .bind(target_id)
    .bind(&note)
    .execute(&mut *tx)
    .await?;
    let row = sqlx::query(&format!(
        "SELECT {LATER_COLUMNS} FROM penfii.later_items WHERE user_id=$1 AND kind=$2 AND target_id=$3 AND state='ACTIVE'"
    ))
    .bind(actor_id)
    .bind(kind)
    .bind(target_id)
    .fetch_one(&mut *tx)
    .await?;
    let _ = inserted;
    let item = later_from(&mut tx, actor_id, &row)
        .await?
        .ok_or(CoreError::NotFound)?;
    tx.commit().await?;
    Ok(item)
}

pub async fn update_later(
    pool: &PgPool,
    actor_id: Uuid,
    later_id: Uuid,
    expected_version: i64,
    state: Option<String>,
    note: Option<Option<String>>,
) -> Result<LaterItem, CoreError> {
    if let Some(state) = &state
        && !matches!(state.as_str(), "ACTIVE" | "DONE" | "ARCHIVED")
    {
        return Err(CoreError::validation(
            "state must be ACTIVE, DONE or ARCHIVED",
        ));
    }
    let mut tx = pool.begin().await?;
    let row = sqlx::query(&format!(
        "SELECT {LATER_COLUMNS} FROM penfii.later_items WHERE id=$1 AND user_id=$2 FOR UPDATE"
    ))
    .bind(later_id)
    .bind(actor_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(CoreError::NotFound)?;
    let version: i64 = row.try_get("version")?;
    if version != expected_version {
        return Err(CoreError::conflict_extra(
            "version_conflict",
            "version conflict",
            serde_json::from_value(serde_json::json!({"currentVersion":version}))
                .unwrap_or_default(),
        ));
    }
    let note = match note {
        Some(note) => note_of(note)?,
        None => row.try_get("note")?,
    };
    let state = state.unwrap_or(row.try_get("state")?);
    let updated = sqlx::query(
        "UPDATE penfii.later_items SET state=$2,note=$3,version=version+1,updated_at=now() WHERE id=$1",
    )
    .bind(later_id)
    .bind(&state)
    .bind(&note)
    .execute(&mut *tx)
    .await;
    if let Err(error) = updated {
        // Bringing a finished card back while the same object is saved again.
        if error
            .as_database_error()
            .is_some_and(|db| db.is_unique_violation())
        {
            return Err(CoreError::conflict(
                "later_exists",
                "this object is already saved for later",
            ));
        }
        return Err(error.into());
    }
    if state != "ACTIVE" {
        crate::reminders::cancel_for_later(&mut tx, later_id, "LATER_CLOSED").await?;
    }
    let row = sqlx::query(&format!(
        "SELECT {LATER_COLUMNS} FROM penfii.later_items WHERE id=$1"
    ))
    .bind(later_id)
    .fetch_one(&mut *tx)
    .await?;
    let item = later_from(&mut tx, actor_id, &row)
        .await?
        .ok_or(CoreError::NotFound)?;
    tx.commit().await?;
    Ok(item)
}

pub async fn remove_later(pool: &PgPool, actor_id: Uuid, later_id: Uuid) -> Result<(), CoreError> {
    let deleted = sqlx::query("DELETE FROM penfii.later_items WHERE id=$1 AND user_id=$2")
        .bind(later_id)
        .bind(actor_id)
        .execute(pool)
        .await?;
    if deleted.rows_affected() == 0 {
        return Err(CoreError::NotFound);
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Pins
// ---------------------------------------------------------------------------------------------

pub async fn list_pins(
    pool: &PgPool,
    actor_id: Uuid,
    project_id: Uuid,
) -> Result<ProjectPins, CoreError> {
    let mut tx = pool.begin().await?;
    let (_, role) = require_project_role(&mut tx, project_id, actor_id, false).await?;
    let rows = sqlx::query(
        "SELECT p.id,p.kind,p.url,p.title,p.position,p.pinned_by,p.created_at,
                t.id AS ticket_id,t.public_key,t.title AS ticket_title,t.hierarchy_kind,t.channel_id,t.archived_at,t.trashed_at,t.purged_at
         FROM penfii.project_pins p LEFT JOIN penfii.tickets t ON t.id=p.ticket_id
         WHERE p.project_id=$1 ORDER BY p.position,p.id",
    )
    .bind(project_id)
    .fetch_all(&mut *tx)
    .await?;
    let mut pins = Vec::new();
    for row in rows {
        let kind: String = row.try_get("kind")?;
        let mut ticket = None;
        let mut title: Option<String> = row.try_get("title")?;
        if kind == "TICKET" {
            let gone = row
                .try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("archived_at")?
                .is_some()
                || row
                    .try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("trashed_at")?
                    .is_some()
                || row
                    .try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("purged_at")?
                    .is_some();
            // A Ticket the viewer cannot open is not in the list for that viewer.
            if gone || !channel_allowed(&mut tx, row.try_get("channel_id")?, actor_id).await? {
                continue;
            }
            let key: String = row.try_get("public_key")?;
            let ticket_title: String = row.try_get("ticket_title")?;
            title = Some(title.unwrap_or_else(|| format!("{key} {ticket_title}")));
            ticket = Some(TicketRef {
                id: row.try_get("ticket_id")?,
                key,
                title: ticket_title,
                hierarchy_kind: row.try_get("hierarchy_kind")?,
            });
        }
        pins.push(PinItem {
            id: row.try_get("id")?,
            kind,
            title: title.unwrap_or_default(),
            url: row.try_get("url")?,
            ticket,
            position: row.try_get("position")?,
            pinned_by: row.try_get("pinned_by")?,
            created_at: row.try_get("created_at")?,
        });
    }
    let can_manage = crate::project_role::can_manage_or_designated(
        &mut tx,
        project_id,
        actor_id,
        &role,
        crate::project_role::Designation::ManagePins,
    )
    .await?;
    tx.commit().await?;
    Ok(ProjectPins { can_manage, pins })
}

pub async fn add_pin(
    pool: &PgPool,
    actor_id: Uuid,
    project_id: Uuid,
    ticket_id: Option<Uuid>,
    url: Option<String>,
    title: Option<String>,
) -> Result<ProjectPins, CoreError> {
    let title = title.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty());
    if title.as_ref().is_some_and(|t| t.chars().count() > 120) {
        return Err(CoreError::validation(
            "a pin title is limited to 120 characters",
        ));
    }
    let mut tx = pool.begin().await?;
    let (workspace_id, role) = require_project_role(&mut tx, project_id, actor_id, false).await?;
    if !crate::project_role::can_manage_or_designated(
        &mut tx,
        project_id,
        actor_id,
        &role,
        crate::project_role::Designation::ManagePins,
    )
    .await?
    {
        return Err(CoreError::forbidden(
            "a project admin or a designated editor pins for everyone",
        ));
    }
    let (kind, url) = match (ticket_id, url) {
        (Some(ticket_id), None) => {
            let row = sqlx::query(
                "SELECT channel_id FROM penfii.tickets WHERE id=$1 AND project_id=$2
                 AND archived_at IS NULL AND trashed_at IS NULL AND purged_at IS NULL",
            )
            .bind(ticket_id)
            .bind(project_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(CoreError::NotFound)?;
            if !channel_allowed(&mut tx, row.try_get("channel_id")?, actor_id).await? {
                return Err(CoreError::NotFound);
            }
            ("TICKET", None)
        }
        (None, Some(url)) => {
            let url = url.trim().to_owned();
            if !(url.starts_with("https://") || url.starts_with("http://"))
                || url.chars().count() > 2000
                || url.chars().any(char::is_whitespace)
            {
                return Err(CoreError::validation(
                    "a pinned link must be an http(s) address",
                ));
            }
            ("URL", Some(url))
        }
        _ => {
            return Err(CoreError::validation(
                "pin either a Ticket or a link, not both",
            ));
        }
    };
    let inserted = sqlx::query(
        "INSERT INTO penfii.project_pins(id,workspace_id,project_id,kind,ticket_id,url,title,position,pinned_by)
         SELECT $1,$2,$3,$4,$5,$6,$7,coalesce(max(position),0)+$8,$9 FROM penfii.project_pins WHERE project_id=$3
         ON CONFLICT DO NOTHING",
    )
    .bind(Uuid::new_v4())
    .bind(workspace_id)
    .bind(project_id)
    .bind(kind)
    .bind(ticket_id)
    .bind(&url)
    .bind(&title)
    .bind(POSITION_STEP)
    .bind(actor_id)
    .execute(&mut *tx)
    .await?;
    if inserted.rows_affected() == 0 {
        return Err(CoreError::conflict("pin_exists", "this is already pinned"));
    }
    tx.commit().await?;
    list_pins(pool, actor_id, project_id).await
}

pub async fn remove_pin(pool: &PgPool, actor_id: Uuid, pin_id: Uuid) -> Result<(), CoreError> {
    let mut tx = pool.begin().await?;
    let project_id: Uuid =
        sqlx::query_scalar("SELECT project_id FROM penfii.project_pins WHERE id=$1 FOR UPDATE")
            .bind(pin_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(CoreError::NotFound)?;
    let (_, role) = require_project_role(&mut tx, project_id, actor_id, false).await?;
    if !crate::project_role::can_manage_or_designated(
        &mut tx,
        project_id,
        actor_id,
        &role,
        crate::project_role::Designation::ManagePins,
    )
    .await?
    {
        return Err(CoreError::forbidden(
            "a project admin or a designated editor removes a pin",
        ));
    }
    sqlx::query("DELETE FROM penfii.project_pins WHERE id=$1")
        .bind(pin_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// A Project admin designates (or takes back) what an editor may do beyond editing: waive
/// dependencies and manage Pins. It lives on the editor's own grant, so it cannot widen what the
/// person can see, and only an editor can hold it.
pub async fn set_designation(
    pool: &PgPool,
    actor_id: Uuid,
    project_id: Uuid,
    user_id: Uuid,
    can_waive_dependencies: bool,
    can_manage_pins: bool,
) -> Result<(bool, bool), CoreError> {
    let mut tx = pool.begin().await?;
    let (_, role) = require_project_role(&mut tx, project_id, actor_id, false).await?;
    if !crate::project_role::can_manage(&role) {
        return Err(CoreError::forbidden(
            "only a project admin designates editors",
        ));
    }
    let current = sqlx::query(
        "SELECT role,can_waive_dependencies,can_manage_pins FROM penfii.project_user_grants WHERE project_id=$1 AND user_id=$2 FOR UPDATE",
    )
    .bind(project_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?;
    let grant_role: Option<String> = current.as_ref().map(|r| r.try_get("role")).transpose()?;
    if grant_role.as_deref() != Some("EDITOR") {
        return Err(CoreError::validation(
            "only a member who is directly an editor of the Project can be designated",
        ));
    }
    sqlx::query(
        "UPDATE penfii.project_user_grants SET can_waive_dependencies=$3, can_manage_pins=$4 WHERE project_id=$1 AND user_id=$2",
    )
    .bind(project_id)
    .bind(user_id)
    .bind(can_waive_dependencies)
    .bind(can_manage_pins)
    .execute(&mut *tx)
    .await?;
    let (was_waive, was_pins): (bool, bool) = current
        .as_ref()
        .map(|r| -> Result<_, sqlx::Error> {
            Ok((
                r.try_get("can_waive_dependencies")?,
                r.try_get("can_manage_pins")?,
            ))
        })
        .transpose()?
        .unwrap_or((false, false));
    // Every real change is recorded: who changed whose capability, from what to what.
    if (was_waive, was_pins) != (can_waive_dependencies, can_manage_pins) {
        let workspace: Uuid =
            sqlx::query_scalar("SELECT workspace_id FROM penfii.projects WHERE id=$1")
                .bind(project_id)
                .fetch_one(&mut *tx)
                .await?;
        sqlx::query(
            "INSERT INTO penfii.audit_entries(id,workspace_id,actor_user_id,action_code,outcome_code,target_kind,target_id,command_id,data)
             VALUES($1,$2,$3,'project.designation.set','OK','PROJECT',$4,$5,$6)",
        )
        .bind(Uuid::new_v4())
        .bind(workspace)
        .bind(actor_id)
        .bind(project_id)
        .bind(Uuid::new_v4())
        .bind(serde_json::json!({
            "userId": user_id,
            "before": {"canWaiveDependencies": was_waive, "canManagePins": was_pins},
            "after": {"canWaiveDependencies": can_waive_dependencies, "canManagePins": can_manage_pins},
        }))
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok((can_waive_dependencies, can_manage_pins))
}

/// What a Project admin sees on the Permissions page: who holds a direct grant and which of them
/// can be designated, the Teams linked to the Project with the role each gives (none: linked only), who reaches it
/// through a Group (read only), and the latest
/// designation changes.
pub async fn permissions(
    pool: &PgPool,
    actor_id: Uuid,
    project_id: Uuid,
) -> Result<serde_json::Value, CoreError> {
    let mut tx = pool.begin().await?;
    let (_, role) = require_project_role(&mut tx, project_id, actor_id, false).await?;
    if !crate::project_role::can_manage(&role) {
        return Err(CoreError::forbidden(
            "only a project admin sees permissions",
        ));
    }
    let members = sqlx::query(
        "SELECT g.user_id, u.display_name, g.role, g.can_waive_dependencies, g.can_manage_pins
         FROM penfii.project_user_grants g JOIN penfii.users u ON u.id = g.user_id
         WHERE g.project_id = $1 ORDER BY u.display_name, g.user_id",
    )
    .bind(project_id)
    .fetch_all(&mut *tx)
    .await?;
    let members: Vec<serde_json::Value> = members
        .iter()
        .map(|row| -> Result<_, sqlx::Error> {
            Ok({
                let role =
                    crate::project_role::normalize(row.try_get::<&str, _>("role")?).to_owned();
                serde_json::json!({
                    "userId": row.try_get::<Uuid, _>("user_id")?,
                    "displayName": row.try_get::<String, _>("display_name")?,
                    "role": role,
                    "source": "DIRECT",
                    "canWaiveDependencies": row.try_get::<bool, _>("can_waive_dependencies")?,
                    "canManagePins": row.try_get::<bool, _>("can_manage_pins")?,
                    "designatable": role == "EDITOR",
                })
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()?;
    let groups = sqlx::query(
        "SELECT g.id, g.name, pg.role,
                (SELECT count(*) FROM penfii.group_members gm WHERE gm.group_id = g.id)
                + (SELECT count(*) FROM penfii.team_members tm WHERE tm.team_id = g.system_team_id) AS member_count
         FROM penfii.project_group_grants pg JOIN penfii.groups g ON g.id = pg.group_id AND g.deleted_at IS NULL
         WHERE pg.project_id = $1 ORDER BY g.name, g.id",
    )
    .bind(project_id)
    .fetch_all(&mut *tx)
    .await?;
    let via_groups: Vec<serde_json::Value> = groups
        .iter()
        .map(|row| -> Result<_, sqlx::Error> {
            Ok({
                serde_json::json!({
                    "groupId": row.try_get::<Uuid, _>("id")?,
                    "name": row.try_get::<String, _>("name")?,
                    "role": row.try_get::<String, _>("role")?,
                    "memberCount": row.try_get::<i64, _>("member_count")?,
                })
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()?;
    let teams = sqlx::query(
        "SELECT t.id, t.name, t.archived_at, pt.access_role, (SELECT count(*) FROM penfii.team_members tm WHERE tm.team_id = t.id) AS member_count
         FROM penfii.project_teams pt JOIN penfii.teams t ON t.id = pt.team_id
         WHERE pt.project_id = $1 ORDER BY t.name, t.id",
    )
    .bind(project_id)
    .fetch_all(&mut *tx)
    .await?;
    let via_teams: Vec<serde_json::Value> = teams
        .iter()
        .map(|row| -> Result<_, sqlx::Error> { Ok({
            serde_json::json!({
                "teamId": row.try_get::<Uuid, _>("id")?,
                "name": row.try_get::<String, _>("name")?,
                "role": row.try_get::<Option<String>, _>("access_role")?,
                "memberCount": row.try_get::<i64, _>("member_count")?,
                "archivedAt": row.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("archived_at")?,
            })
        }) })
        .collect::<Result<Vec<_>, sqlx::Error>>()?;
    let changes = sqlx::query(
        "SELECT a.occurred_at, a.data, actor.display_name AS actor_name, target.display_name AS target_name
         FROM penfii.audit_entries a
         LEFT JOIN penfii.users actor ON actor.id = a.actor_user_id
         LEFT JOIN penfii.users target ON target.id = (a.data->>'userId')::uuid
         WHERE a.target_kind = 'PROJECT' AND a.target_id = $1 AND a.action_code = 'project.designation.set'
         ORDER BY a.occurred_at DESC, a.id DESC LIMIT 20",
    )
    .bind(project_id)
    .fetch_all(&mut *tx)
    .await?;
    let recent: Vec<serde_json::Value> = changes
        .iter()
        .map(|row| -> Result<_, sqlx::Error> {
            Ok({
                serde_json::json!({
                    "at": row.try_get::<chrono::DateTime<chrono::Utc>, _>("occurred_at")?,
                    "actorName": row.try_get::<Option<String>, _>("actor_name")?,
                    "userName": row.try_get::<Option<String>, _>("target_name")?,
                    "userId": row.try_get::<serde_json::Value, _>("data")?["userId"],
                    "before": row.try_get::<serde_json::Value, _>("data")?["before"],
                    "after": row.try_get::<serde_json::Value, _>("data")?["after"],
                })
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()?;
    tx.commit().await?;
    Ok(serde_json::json!({
        "members": members,
        "viaGroups": via_groups,
        "teams": via_teams,
        "capabilities": ["dependency_waiver", "pin_management"],
        "recentChanges": recent,
    }))
}

// ---------------------------------------------------------------------------------------------
// Favorite folders
// ---------------------------------------------------------------------------------------------

const MAX_FOLDERS: i64 = 50;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FavoriteFolder {
    pub id: Uuid,
    pub name: String,
    pub position: i64,
}

fn folder_name(name: &str) -> Result<String, CoreError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 60 || name.chars().any(char::is_control) {
        return Err(CoreError::validation("a folder name is 1 to 60 characters"));
    }
    Ok(name.to_owned())
}

pub async fn list_folders(
    pool: &PgPool,
    actor_id: Uuid,
    workspace_id: Uuid,
) -> Result<Vec<FavoriteFolder>, CoreError> {
    let mut conn = pool.acquire().await?;
    crate::access::require_member(&mut conn, workspace_id, actor_id).await?;
    let rows = sqlx::query("SELECT id,name,position FROM penfii.favorite_folders WHERE user_id=$1 AND workspace_id=$2 ORDER BY position,id")
        .bind(actor_id)
        .bind(workspace_id)
        .fetch_all(&mut *conn)
        .await?;
    rows.iter()
        .map(|r| {
            Ok(FavoriteFolder {
                id: r.try_get("id")?,
                name: r.try_get("name")?,
                position: r.try_get("position")?,
            })
        })
        .collect()
}

pub async fn create_folder(
    pool: &PgPool,
    actor_id: Uuid,
    workspace_id: Uuid,
    name: &str,
) -> Result<FavoriteFolder, CoreError> {
    let name = folder_name(name)?;
    let mut tx = pool.begin().await?;
    crate::access::require_member(&mut tx, workspace_id, actor_id).await?;
    // Serialize one person's folders so the count and the position are consistent.
    sqlx::query("SELECT id FROM penfii.users WHERE id=$1 FOR UPDATE")
        .bind(actor_id)
        .fetch_one(&mut *tx)
        .await?;
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.favorite_folders WHERE user_id=$1 AND workspace_id=$2",
    )
    .bind(actor_id)
    .bind(workspace_id)
    .fetch_one(&mut *tx)
    .await?;
    if count >= MAX_FOLDERS {
        return Err(CoreError::conflict(
            "folder_limit",
            "there is a limit of 50 folders",
        ));
    }
    let id = Uuid::new_v4();
    let inserted = sqlx::query(
        "INSERT INTO penfii.favorite_folders(id,user_id,workspace_id,name,position)
         SELECT $1,$2,$3,$4,coalesce(max(position),0)+$5 FROM penfii.favorite_folders WHERE user_id=$2 AND workspace_id=$3
         RETURNING position",
    )
    .bind(id).bind(actor_id).bind(workspace_id).bind(&name).bind(POSITION_STEP)
    .fetch_one(&mut *tx)
    .await;
    let row = match inserted {
        Ok(row) => row,
        Err(sqlx::Error::Database(db)) if db.is_unique_violation() => {
            return Err(CoreError::conflict(
                "folder_name_taken",
                "a folder with this name already exists",
            ));
        }
        Err(error) => return Err(error.into()),
    };
    tx.commit().await?;
    Ok(FavoriteFolder {
        id,
        name,
        position: row.try_get("position")?,
    })
}

pub async fn rename_folder(
    pool: &PgPool,
    actor_id: Uuid,
    folder_id: Uuid,
    name: &str,
) -> Result<FavoriteFolder, CoreError> {
    let name = folder_name(name)?;
    let updated = sqlx::query(
        "UPDATE penfii.favorite_folders SET name=$3 WHERE id=$1 AND user_id=$2 RETURNING position",
    )
    .bind(folder_id)
    .bind(actor_id)
    .bind(&name)
    .fetch_optional(pool)
    .await;
    match updated {
        Ok(Some(row)) => Ok(FavoriteFolder {
            id: folder_id,
            name,
            position: row.try_get("position")?,
        }),
        Ok(None) => Err(CoreError::NotFound),
        Err(sqlx::Error::Database(db)) if db.is_unique_violation() => Err(CoreError::conflict(
            "folder_name_taken",
            "a folder with this name already exists",
        )),
        Err(error) => Err(error.into()),
    }
}

pub async fn order_folders(
    pool: &PgPool,
    actor_id: Uuid,
    workspace_id: Uuid,
    ids: Vec<Uuid>,
) -> Result<(), CoreError> {
    let mut tx = pool.begin().await?;
    crate::access::require_member(&mut tx, workspace_id, actor_id).await?;
    for (index, id) in ids.iter().enumerate() {
        sqlx::query("UPDATE penfii.favorite_folders SET position=$4 WHERE id=$1 AND user_id=$2 AND workspace_id=$3")
            .bind(id).bind(actor_id).bind(workspace_id).bind((index as i64 + 1) * POSITION_STEP)
            .execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Deletes a folder. Its Favorites are not deleted: they go to the top level, after the Favorites
/// already there, in the order they had inside the folder.
pub async fn delete_folder(
    pool: &PgPool,
    actor_id: Uuid,
    folder_id: Uuid,
) -> Result<i64, CoreError> {
    let mut tx = pool.begin().await?;
    let workspace: Uuid = sqlx::query_scalar(
        "SELECT workspace_id FROM penfii.favorite_folders WHERE id=$1 AND user_id=$2 FOR UPDATE",
    )
    .bind(folder_id)
    .bind(actor_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(CoreError::NotFound)?;
    let moved = sqlx::query(
        "UPDATE penfii.favorites f SET folder_id=NULL, position=base.top + r.rn*$4
         FROM (SELECT kind,target_id,row_number() OVER (ORDER BY position,created_at,target_id) AS rn
               FROM penfii.favorites WHERE user_id=$1 AND folder_id=$2) r,
              (SELECT coalesce(max(position),0) AS top FROM penfii.favorites WHERE user_id=$1 AND workspace_id=$3 AND folder_id IS NULL) base
         WHERE f.user_id=$1 AND f.kind=r.kind AND f.target_id=r.target_id AND f.folder_id=$2",
    )
    .bind(actor_id).bind(folder_id).bind(workspace).bind(POSITION_STEP)
    .execute(&mut *tx).await?
    .rows_affected();
    sqlx::query("DELETE FROM penfii.favorite_folders WHERE id=$1")
        .bind(folder_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(moved as i64)
}

/// Puts a Favorite in a folder, or back at the top level with `None`.
pub async fn move_favorite(
    pool: &PgPool,
    actor_id: Uuid,
    workspace_id: Uuid,
    kind: &str,
    target_id: Uuid,
    folder_id: Option<Uuid>,
) -> Result<(), CoreError> {
    let mut tx = pool.begin().await?;
    crate::access::require_member(&mut tx, workspace_id, actor_id).await?;
    if let Some(folder) = folder_id {
        let own: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM penfii.favorite_folders WHERE id=$1 AND user_id=$2 AND workspace_id=$3)")
            .bind(folder).bind(actor_id).bind(workspace_id).fetch_one(&mut *tx).await?;
        if !own {
            return Err(CoreError::NotFound);
        }
    }
    let moved = sqlx::query("UPDATE penfii.favorites SET folder_id=$5 WHERE user_id=$1 AND workspace_id=$2 AND kind=$3 AND target_id=$4")
        .bind(actor_id).bind(workspace_id).bind(kind).bind(target_id).bind(folder_id)
        .execute(&mut *tx).await?
        .rows_affected();
    if moved == 0 {
        return Err(CoreError::NotFound);
    }
    tx.commit().await?;
    Ok(())
}
