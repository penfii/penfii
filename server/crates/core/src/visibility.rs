//! One place that answers "may this person see these objects?".
//!
//! Search, Inbox, Later, Reminder and Watch all ask here. Each function answers from the same
//! rules the screens use (`project_role::project_role`, `project_role::channel_allowed`, `access::load_conversation`
//! and the Space listing; the last three share `access::CHANNEL_READABLE`), per container and cached for
//! the call, so the rule is never copied into another SQL string. An object the person cannot see is simply absent from the answer.

use std::collections::{HashMap, HashSet};

use sqlx::{PgConnection, Row};
use uuid::Uuid;

use crate::access;
use crate::error::CoreError;
use crate::project_role::{channel_allowed, project_role};

struct Cache<'a> {
    actor: Uuid,
    projects: HashMap<Uuid, bool>,
    channels: HashMap<Option<Uuid>, bool>,
    conversations: HashMap<Uuid, bool>,
    workspaces: HashMap<Uuid, bool>,
    conn: &'a mut PgConnection,
}

impl<'a> Cache<'a> {
    fn new(conn: &'a mut PgConnection, actor: Uuid) -> Self {
        Self {
            actor,
            projects: HashMap::new(),
            channels: HashMap::new(),
            conversations: HashMap::new(),
            workspaces: HashMap::new(),
            conn,
        }
    }

    async fn project(&mut self, project_id: Uuid) -> Result<bool, CoreError> {
        if let Some(known) = self.projects.get(&project_id) {
            return Ok(*known);
        }
        let allowed = match project_role(self.conn, project_id, self.actor).await {
            Ok((_, role)) => role.is_some(),
            Err(CoreError::NotFound) => false,
            Err(other) => return Err(other),
        };
        self.projects.insert(project_id, allowed);
        Ok(allowed)
    }

    /// The gate a Ticket's origin channel puts on it.
    async fn ticket_channel(&mut self, channel_id: Option<Uuid>) -> Result<bool, CoreError> {
        if let Some(known) = self.channels.get(&channel_id) {
            return Ok(*known);
        }
        let allowed = channel_allowed(self.conn, channel_id, self.actor).await?;
        self.channels.insert(channel_id, allowed);
        Ok(allowed)
    }

    async fn conversation(&mut self, conversation_id: Uuid) -> Result<bool, CoreError> {
        if let Some(known) = self.conversations.get(&conversation_id) {
            return Ok(*known);
        }
        let allowed = access::load_conversation(self.conn, self.actor, conversation_id)
            .await?
            .is_some_and(|access| access.can_read());
        self.conversations.insert(conversation_id, allowed);
        Ok(allowed)
    }

    async fn member(&mut self, workspace_id: Uuid) -> Result<bool, CoreError> {
        if let Some(known) = self.workspaces.get(&workspace_id) {
            return Ok(*known);
        }
        let allowed = access::workspace_role(self.conn, workspace_id, self.actor)
            .await?
            .is_some();
        self.workspaces.insert(workspace_id, allowed);
        Ok(allowed)
    }
}

/// Tickets the person may open: live or archived (an archived Ticket is read-only but readable, with
/// its links and subscription), never trashed or purged; whose Project the person can read and
/// whose origin channel, if private, the person may enter.
pub async fn tickets(
    conn: &mut PgConnection,
    actor: Uuid,
    ids: &[Uuid],
) -> Result<HashSet<Uuid>, CoreError> {
    if ids.is_empty() {
        return Ok(HashSet::new());
    }
    let rows = sqlx::query(
        "SELECT id, project_id, channel_id FROM penfii.tickets
         WHERE id = ANY($1) AND trashed_at IS NULL AND purged_at IS NULL",
    )
    .bind(ids)
    .fetch_all(&mut *conn)
    .await?;
    let mut cache = Cache::new(conn, actor);
    let mut visible = HashSet::new();
    for row in rows {
        let (id, project, channel): (Uuid, Uuid, Option<Uuid>) = (
            row.try_get("id")?,
            row.try_get("project_id")?,
            row.try_get("channel_id")?,
        );
        if cache.project(project).await? && cache.ticket_channel(channel).await? {
            visible.insert(id);
        }
    }
    Ok(visible)
}

/// Messages that are not deleted, in a conversation the person can read.
pub async fn messages(
    conn: &mut PgConnection,
    actor: Uuid,
    ids: &[Uuid],
) -> Result<HashSet<Uuid>, CoreError> {
    if ids.is_empty() {
        return Ok(HashSet::new());
    }
    let rows = sqlx::query(
        "SELECT id, conversation_id FROM penfii.messages WHERE id = ANY($1) AND deleted_at IS NULL",
    )
    .bind(ids)
    .fetch_all(&mut *conn)
    .await?;
    let mut cache = Cache::new(conn, actor);
    let mut visible = HashSet::new();
    for row in rows {
        let (id, conversation): (Uuid, Uuid) =
            (row.try_get("id")?, row.try_get("conversation_id")?);
        if cache.conversation(conversation).await? {
            visible.insert(id);
        }
    }
    Ok(visible)
}

/// A File is visible when it is available, not deleted, in a Workspace the person belongs to, and
/// either attached to a live message the person can read, or to a live Ticket they can open, or the
/// person's own upload that is not attached yet. Attachments of older message versions give no access.
pub async fn files(
    conn: &mut PgConnection,
    actor: Uuid,
    ids: &[Uuid],
) -> Result<HashSet<Uuid>, CoreError> {
    if ids.is_empty() {
        return Ok(HashSet::new());
    }
    let files = sqlx::query(
        "SELECT id, workspace_id, created_by FROM penfii.files
         WHERE id = ANY($1) AND state = 'AVAILABLE' AND deleted_at IS NULL",
    )
    .bind(ids)
    .fetch_all(&mut *conn)
    .await?;
    let attachments = sqlx::query(
        "SELECT a.file_id, m.conversation_id
         FROM penfii.message_attachments a JOIN penfii.messages m ON m.id = a.message_id
         WHERE a.file_id = ANY($1) AND m.deleted_at IS NULL",
    )
    .bind(ids)
    .fetch_all(&mut *conn)
    .await?;
    let mut attached_anywhere: HashSet<Uuid> = sqlx::query_scalar(
        "SELECT DISTINCT file_id FROM penfii.message_attachments WHERE file_id = ANY($1)",
    )
    .bind(ids)
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .collect();
    // A file on a Ticket is as visible as the live Ticket; on any other Ticket state it is not
    // the uploader's loose upload either.
    let on_tickets: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT file_id, ticket_id FROM penfii.ticket_attachments WHERE file_id = ANY($1)",
    )
    .bind(ids)
    .fetch_all(&mut *conn)
    .await?;
    let ticket_ids: Vec<Uuid> = on_tickets.iter().map(|(_, ticket)| *ticket).collect();
    let open_tickets = tickets(conn, actor, &ticket_ids).await?;
    let mut ticket_files = HashSet::new();
    for (file, ticket) in on_tickets {
        attached_anywhere.insert(file);
        if open_tickets.contains(&ticket) {
            ticket_files.insert(file);
        }
    }
    let in_knowledge = attached_to_knowledge(conn, ids).await?;
    let mut by_file: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
    for row in attachments {
        by_file
            .entry(row.try_get("file_id")?)
            .or_default()
            .push(row.try_get("conversation_id")?);
    }
    let knowledge = knowledge_files(conn, actor, ids).await?;
    let mut cache = Cache::new(conn, actor);
    let mut visible = HashSet::new();
    for row in files {
        let (id, workspace, owner): (Uuid, Uuid, Uuid) = (
            row.try_get("id")?,
            row.try_get("workspace_id")?,
            row.try_get("created_by")?,
        );
        if !cache.member(workspace).await? {
            continue;
        }
        // A File in a Doc is readable by whoever can read that Doc.
        if knowledge.contains(&id) {
            visible.insert(id);
            continue;
        }
        if ticket_files.contains(&id) {
            visible.insert(id);
            continue;
        }
        if !attached_anywhere.contains(&id) && !in_knowledge.contains(&id) {
            if owner == actor {
                visible.insert(id);
            }
            continue;
        }
        for conversation in by_file.get(&id).into_iter().flatten() {
            if cache.conversation(*conversation).await? {
                visible.insert(id);
                break;
            }
        }
    }
    Ok(visible)
}

/// Channels as the Space list shows them: a member, or a public channel that belongs to no Project or
/// Team, or one whose Project (and Team) the person is in. Archived channels follow the same rule.
pub async fn channels(
    conn: &mut PgConnection,
    actor: Uuid,
    ids: &[Uuid],
) -> Result<HashSet<Uuid>, CoreError> {
    if ids.is_empty() {
        return Ok(HashSet::new());
    }
    let rows: Vec<Uuid> = sqlx::query_scalar(&format!(
        "SELECT ch.id
         FROM penfii.channels ch
         JOIN penfii.conversations c ON c.id = ch.id
         JOIN penfii.workspace_members wm ON wm.workspace_id = c.workspace_id AND wm.user_id = $2 AND wm.state = 'ACTIVE'
         LEFT JOIN penfii.channel_members cm ON cm.channel_id = ch.id AND cm.user_id = $2
         WHERE ch.id = ANY($1) AND {}",
        access::CHANNEL_READABLE
    ))
    .bind(ids)
    .bind(actor)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().collect())
}

/// Projects and Teams are listed to every active member of their Workspace.
pub async fn projects(
    conn: &mut PgConnection,
    actor: Uuid,
    ids: &[Uuid],
) -> Result<HashSet<Uuid>, CoreError> {
    workspace_listed(conn, actor, "projects", ids).await
}

pub async fn teams(
    conn: &mut PgConnection,
    actor: Uuid,
    ids: &[Uuid],
) -> Result<HashSet<Uuid>, CoreError> {
    workspace_listed(conn, actor, "teams", ids).await
}

async fn workspace_listed(
    conn: &mut PgConnection,
    actor: Uuid,
    table: &str,
    ids: &[Uuid],
) -> Result<HashSet<Uuid>, CoreError> {
    if ids.is_empty() {
        return Ok(HashSet::new());
    }
    let rows: Vec<Uuid> = sqlx::query_scalar(&format!(
        "SELECT t.id FROM penfii.{table} t
         JOIN penfii.workspace_members wm ON wm.workspace_id = t.workspace_id AND wm.user_id = $2 AND wm.state = 'ACTIVE'
         WHERE t.id = ANY($1)"
    ))
    .bind(ids)
    .bind(actor)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().collect())
}

/// Saved Views: a personal View is its owner's; a Project-shared View follows the Project.
pub async fn views(
    conn: &mut PgConnection,
    actor: Uuid,
    ids: &[Uuid],
) -> Result<HashSet<Uuid>, CoreError> {
    if ids.is_empty() {
        return Ok(HashSet::new());
    }
    let rows = sqlx::query(
        "SELECT id, scope, owner_id, project_id, workspace_id FROM penfii.project_views
         WHERE id = ANY($1) AND deleted_at IS NULL",
    )
    .bind(ids)
    .fetch_all(&mut *conn)
    .await?;
    let mut cache = Cache::new(conn, actor);
    let mut visible = HashSet::new();
    for row in rows {
        let id: Uuid = row.try_get("id")?;
        let scope: String = row.try_get("scope")?;
        let owner: Uuid = row.try_get("owner_id")?;
        let project: Option<Uuid> = row.try_get("project_id")?;
        let workspace: Uuid = row.try_get("workspace_id")?;
        let allowed = if scope == "PERSONAL" {
            owner == actor && cache.member(workspace).await?
        } else {
            match project {
                Some(project) => cache.project(project).await?,
                None => false,
            }
        };
        if allowed {
            visible.insert(id);
        }
    }
    Ok(visible)
}

// ---------------------------------------------------------------------------------------------
// Docs
// ---------------------------------------------------------------------------------------------

/// What a person may do with a Doc page. The rules live here and nowhere
/// else: HTTP, Search, the co-editing socket, attachments and Referenced by all ask this function.
/// `manage` implies `edit` and `edit` implies `read`; `edit` can be narrower than `read` through a
/// `VIEWER` participant, who reads but does not edit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DocRights {
    pub read: bool,
    pub edit: bool,
    pub manage: bool,
}

/// The rights a share scope gives (pages).
///
/// * `WORKSPACE`: every active member reads and edits; `MANAGER` participants, the creator and
///   Workspace OWNER/ADMIN manage.
/// * `PROJECT`: the people of the scope's Project (and, when one is named, of that Team too: the
///   intersection) read and edit, and so do invited participants; only a `MANAGER` participant manages.
///   Being in a Team without being in the Project gives nothing, and Workspace OWNER/ADMIN get nothing extra.
/// * `RESTRICTED` (private): invited participants only; only a `MANAGER` participant manages.
///
/// A `VIEWER` participant row narrows editing to reading in every scope (unless the person manages).
fn share_rights(
    visibility: &str,
    creator: Uuid,
    actor: Uuid,
    workspace_role: &str,
    participant: Option<&str>,
    in_scope: bool,
) -> DocRights {
    let (read, manage) = match visibility {
        "WORKSPACE" => (
            true,
            participant == Some("MANAGER")
                || creator == actor
                || matches!(workspace_role, "OWNER" | "ADMIN"),
        ),
        "PROJECT" => (
            in_scope || participant.is_some(),
            participant == Some("MANAGER"),
        ),
        _ => (participant.is_some(), participant == Some("MANAGER")),
    };
    DocRights {
        read,
        edit: manage || (read && participant != Some("VIEWER")),
        manage,
    }
}

pub async fn doc_rights(
    conn: &mut PgConnection,
    actor: Uuid,
    ids: &[Uuid],
) -> Result<HashMap<Uuid, DocRights>, CoreError> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = sqlx::query(
        "SELECT p.id, p.visibility, p.created_by, wm.role AS workspace_role, pp.access_role,
                penfii.in_share_scope(p.scope_project_id, p.scope_team_id, $2) AS in_scope
         FROM penfii.doc_pages p
         JOIN penfii.workspace_members wm
           ON wm.workspace_id = p.workspace_id AND wm.user_id = $2 AND wm.state = 'ACTIVE'
         LEFT JOIN penfii.doc_page_participants pp ON pp.page_id = p.id AND pp.user_id = $2
         WHERE p.id = ANY($1) AND p.deleted_at IS NULL",
    )
    .bind(ids)
    .bind(actor)
    .fetch_all(&mut *conn)
    .await?;
    let mut out = HashMap::new();
    for row in rows {
        let id: Uuid = row.try_get("id")?;
        let visibility: String = row.try_get("visibility")?;
        let creator: Uuid = row.try_get("created_by")?;
        let workspace_role: String = row.try_get("workspace_role")?;
        let participant: Option<String> = row.try_get("access_role")?;
        let in_scope: bool = row.try_get("in_scope")?;
        let rights = share_rights(
            &visibility,
            creator,
            actor,
            &workspace_role,
            participant.as_deref(),
            in_scope,
        );
        out.insert(id, rights);
    }
    Ok(out)
}

/// Doc pages the person may read. Deleted pages and pages of other Workspaces are absent.
pub async fn docs(
    conn: &mut PgConnection,
    actor: Uuid,
    ids: &[Uuid],
) -> Result<HashSet<Uuid>, CoreError> {
    Ok(doc_rights(conn, actor, ids)
        .await?
        .into_iter()
        .filter(|(_, rights)| rights.read)
        .map(|(id, _)| id)
        .collect())
}

/// Files that Docs make readable: a File attached to a live Doc page the person can read.
pub async fn doc_files(
    conn: &mut PgConnection,
    actor: Uuid,
    ids: &[Uuid],
) -> Result<HashSet<Uuid>, CoreError> {
    if ids.is_empty() {
        return Ok(HashSet::new());
    }
    let rows = sqlx::query(
        "SELECT DISTINCT df.file_id, df.page_id
         FROM penfii.doc_page_files df
         JOIN penfii.files f ON f.id = df.file_id AND f.state = 'AVAILABLE' AND f.deleted_at IS NULL
         WHERE df.file_id = ANY($1)",
    )
    .bind(ids)
    .fetch_all(&mut *conn)
    .await?;
    let pages: Vec<Uuid> = rows
        .iter()
        .map(|r| r.try_get("page_id"))
        .collect::<Result<_, _>>()?;
    let readable = docs(conn, actor, &pages).await?;
    let mut out = HashSet::new();
    for row in rows {
        let page: Uuid = row.try_get("page_id")?;
        if readable.contains(&page) {
            out.insert(row.try_get("file_id")?);
        }
    }
    Ok(out)
}

/// Files that are attached to a Doc page at all (readable or not). Such a file
/// is no longer "the uploader's own unattached upload": its readers are the Doc's.
pub async fn attached_to_knowledge(
    conn: &mut PgConnection,
    ids: &[Uuid],
) -> Result<HashSet<Uuid>, CoreError> {
    if ids.is_empty() {
        return Ok(HashSet::new());
    }
    let rows: Vec<Uuid> =
        sqlx::query_scalar("SELECT file_id FROM penfii.doc_page_files WHERE file_id = ANY($1)")
            .bind(ids)
            .fetch_all(&mut *conn)
            .await?;
    Ok(rows.into_iter().collect())
}

/// Readable through a Doc page (the clause D12 adds to `files`).
pub async fn knowledge_files(
    conn: &mut PgConnection,
    actor: Uuid,
    ids: &[Uuid],
) -> Result<HashSet<Uuid>, CoreError> {
    doc_files(conn, actor, ids).await
}

/// Checks the scope fields that go with a share `visibility` and returns them as they are stored.
/// `PROJECT` needs a Project of the Workspace; a Team may be named only if it is one of that Project's
/// Teams (it narrows the Project's people). The other scopes carry neither. Sharing into a Project is
/// for its participants: a Project's inside is not open to anyone else, to write into or to hand out.
pub async fn check_share_scope(
    conn: &mut PgConnection,
    actor: Uuid,
    workspace_id: Uuid,
    visibility: &str,
    project: Option<Uuid>,
    team: Option<Uuid>,
) -> Result<(Option<Uuid>, Option<Uuid>), CoreError> {
    if visibility != "PROJECT" {
        if project.is_some() || team.is_some() {
            return Err(CoreError::validation(
                "a Project or Team scope goes with visibility PROJECT only",
            ));
        }
        return Ok((None, None));
    }
    let Some(project) = project else {
        return Err(CoreError::validation("visibility PROJECT needs a Project"));
    };
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
    crate::access::require_project_participant(conn, project, actor).await?;
    if let Some(team) = team {
        let linked: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM penfii.project_teams WHERE project_id=$1 AND team_id=$2)",
        )
        .bind(project)
        .bind(team)
        .fetch_one(&mut *conn)
        .await?;
        if !linked {
            return Err(CoreError::validation(
                "the Team is not one of the Project's Teams",
            ));
        }
    }
    Ok((Some(project), team))
}
