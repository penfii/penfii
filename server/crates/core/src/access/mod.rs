//! Visibility checks. Hidden channels, DMs, and messages are not found.

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use uuid::Uuid;

use crate::error::CoreError;

#[derive(Debug, Clone)]
pub struct ConvAccess {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub kind: String,
    pub last_message_seq: i64,
    pub last_change_seq: i64,
    pub replay_floor_seq: i64,
    pub created_at: DateTime<Utc>,
    pub visibility: Option<String>,
    pub project_id: Option<Uuid>,
    pub team_id: Option<Uuid>,
    pub channel_role: Option<String>,
    pub workspace_role: Option<String>,
    pub dm_member: bool,
    pub project_member: bool,
    /// [`CHANNEL_READABLE`] for this person; `false` for a DM.
    pub channel_readable: bool,
    /// The channel belongs to a Team that is archived. Derived from the Team on every read, so the
    /// channel row itself is never touched and restoring the Team makes it writable again.
    pub team_archived: bool,
}

/// The channel row of the reader counts only while it is allowed to: in a Project's channel, the
/// reader still has to be one of the Project's people (and the Team's, for a Team's channel). A
/// `penfii.channel_members cm` row left behind by a participation that is gone (a removed grant, an
/// unlinked Team, a Team the person left) grants nothing: not reading, not writing, not a role.
/// SQL over `penfii.channels ch` and `cm` (joined on `cm.channel_id = ch.id AND cm.user_id = $2`).
macro_rules! channel_member_sql {
    () => {
        "(cm.user_id IS NOT NULL
    AND (ch.project_id IS NULL OR penfii.in_share_scope(ch.project_id, ch.team_id, $2)))"
    };
}

/// [`channel_member_sql`] as a condition: this person is a member of the channel with effect.
#[allow(dead_code)]
pub(crate) const CHANNEL_MEMBER: &str = channel_member_sql!();

/// The reader's channel role (`MEMBER` / `MANAGER`) with effect: NULL without an effective membership,
/// so a Manager row of someone who lost the Project is no Manager. Every Manager check reads this
/// instead of `cm.role`.
pub(crate) const CHANNEL_ROLE: &str =
    concat!("(CASE WHEN ", channel_member_sql!(), " THEN cm.role END)");

/// Who may read a channel, as one SQL condition over `penfii.channels ch` and the reader's row
/// `penfii.channel_members cm` (joined on `cm.channel_id = ch.id AND cm.user_id = $2`): an effective
/// member of the channel ([`CHANNEL_MEMBER`]), or anyone for a public channel of no Project or Team,
/// or the Project's (and Team's) people for a public channel of one. Active Workspace membership is
/// checked next to it by each caller. [`ConvAccess::can_read`], `visibility::channels`, the Space
/// listing and `project_role::channel_allowed` (the gate of a Ticket's origin channel) all read this
/// one string.
pub(crate) const CHANNEL_READABLE: &str = concat!(
    "(",
    channel_member_sql!(),
    "
    OR (ch.visibility = 'PUBLIC'
        AND ((ch.project_id IS NULL AND ch.team_id IS NULL)
             OR penfii.in_share_scope(ch.project_id, ch.team_id, $2))))"
);

impl ConvAccess {
    pub fn can_read(&self) -> bool {
        // Every conversation, DMs and private channels included, is closed to someone who is no
        // longer an active member of the Workspace.
        if self.workspace_role.is_none() {
            return false;
        }
        if self.kind == "DM" {
            return self.dm_member;
        }
        self.channel_readable
    }

    /// A channel of an archived Team is read-only: nothing is posted, edited, deleted or reacted to.
    pub fn require_writable(&self) -> Result<(), CoreError> {
        if self.team_archived {
            return Err(CoreError::conflict(
                "channel_read_only",
                "the Team is archived, so its channels are read-only",
            ));
        }
        Ok(())
    }

    pub fn can_write(&self) -> bool {
        if self.workspace_role.is_none() || self.team_archived {
            return false;
        }
        if self.kind == "DM" {
            return self.dm_member;
        }
        if self.channel_role.is_some() {
            return true;
        }
        self.visibility.as_deref() == Some("PUBLIC")
            && (self.project_id.is_some() || self.team_id.is_some())
            && self.project_member
    }
}

struct ConvRow {
    id: Uuid,
    workspace_id: Uuid,
    kind: String,
    last_message_seq: i64,
    last_change_seq: i64,
    replay_floor_seq: i64,
    created_at: DateTime<Utc>,
    visibility: Option<String>,
    project_id: Option<Uuid>,
    team_id: Option<Uuid>,
    channel_role: Option<String>,
    workspace_role: Option<String>,
    dm_member: bool,
    project_member: bool,
    channel_readable: bool,
    team_archived: bool,
}

pub async fn load_conversation(
    conn: &mut PgConnection,
    user_id: Uuid,
    conversation_id: Uuid,
) -> Result<Option<ConvAccess>, CoreError> {
    let row = sqlx::query_as::<_, ConvRow>(&format!(
        "SELECT c.id, c.workspace_id, c.kind, c.last_message_seq, c.last_change_seq,
                c.replay_floor_seq, c.created_at, ch.visibility, ch.project_id, ch.team_id,
                {CHANNEL_ROLE} AS channel_role,
                wm.role AS workspace_role, (dm.user_id IS NOT NULL) AS dm_member,
                (ch.project_id IS NOT NULL AND penfii.in_share_scope(ch.project_id, ch.team_id, $2)
                ) AS project_member,
                COALESCE({CHANNEL_READABLE}, false) AS channel_readable,
                EXISTS (SELECT 1 FROM penfii.teams tt
                        WHERE tt.id = ch.team_id AND tt.archived_at IS NOT NULL) AS team_archived
         FROM penfii.conversations c
         LEFT JOIN penfii.channels ch ON ch.id = c.id
         LEFT JOIN penfii.channel_members cm
            ON cm.channel_id = c.id AND cm.user_id = $2
         LEFT JOIN penfii.workspace_members wm
            ON wm.workspace_id = c.workspace_id AND wm.user_id = $2 AND wm.state = 'ACTIVE'
         LEFT JOIN penfii.dm_members dm
            ON dm.conversation_id = c.id AND dm.user_id = $2
         WHERE c.id = $1",
    ))
    .bind(conversation_id)
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.map(|row| ConvAccess {
        id: row.id,
        workspace_id: row.workspace_id,
        kind: row.kind,
        last_message_seq: row.last_message_seq,
        last_change_seq: row.last_change_seq,
        replay_floor_seq: row.replay_floor_seq,
        created_at: row.created_at,
        visibility: row.visibility,
        project_id: row.project_id,
        team_id: row.team_id,
        channel_role: row.channel_role,
        workspace_role: row.workspace_role,
        dm_member: row.dm_member,
        project_member: row.project_member,
        channel_readable: row.channel_readable,
        team_archived: row.team_archived,
    }))
}

/// Whether the person is in the Project's people and, when a Team is named, in that Team too.
/// The definition lives in the database (`penfii.in_share_scope`) so every read path shares it.
pub async fn in_share_scope(
    conn: &mut PgConnection,
    project_id: Uuid,
    team_id: Option<Uuid>,
    user_id: Uuid,
) -> Result<bool, CoreError> {
    Ok(
        sqlx::query_scalar("SELECT penfii.in_share_scope($1, $2, $3)")
            .bind(project_id)
            .bind(team_id)
            .bind(user_id)
            .fetch_one(&mut *conn)
            .await?,
    )
}

/// The inside of a Project (its Docs, Databases, Channels, Work) is for its participants. A Workspace member
/// who does not take part gets the same "not found" as for a Project that does not exist.
pub async fn require_project_participant(
    conn: &mut PgConnection,
    project_id: Uuid,
    user_id: Uuid,
) -> Result<(), CoreError> {
    if in_share_scope(conn, project_id, None, user_id).await? {
        Ok(())
    } else {
        Err(CoreError::NotFound)
    }
}

pub async fn require_read(
    conn: &mut PgConnection,
    user_id: Uuid,
    conversation_id: Uuid,
) -> Result<ConvAccess, CoreError> {
    match load_conversation(conn, user_id, conversation_id).await? {
        Some(access) if access.can_read() => Ok(access),
        _ => Err(CoreError::NotFound),
    }
}

pub async fn workspace_role(
    conn: &mut PgConnection,
    workspace_id: Uuid,
    user_id: Uuid,
) -> Result<Option<String>, CoreError> {
    let role = sqlx::query_scalar::<_, String>(
        "SELECT role FROM penfii.workspace_members
         WHERE workspace_id = $1 AND user_id = $2 AND state = 'ACTIVE'",
    )
    .bind(workspace_id)
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(role)
}

pub async fn require_member(
    conn: &mut PgConnection,
    workspace_id: Uuid,
    user_id: Uuid,
) -> Result<String, CoreError> {
    workspace_role(conn, workspace_id, user_id)
        .await?
        .ok_or(CoreError::NotFound)
}

impl sqlx::FromRow<'_, sqlx::postgres::PgRow> for ConvRow {
    fn from_row(row: &sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;
        Ok(Self {
            id: row.try_get("id")?,
            workspace_id: row.try_get("workspace_id")?,
            kind: row.try_get("kind")?,
            last_message_seq: row.try_get("last_message_seq")?,
            last_change_seq: row.try_get("last_change_seq")?,
            replay_floor_seq: row.try_get("replay_floor_seq")?,
            created_at: row.try_get("created_at")?,
            visibility: row.try_get("visibility")?,
            project_id: row.try_get("project_id")?,
            team_id: row.try_get("team_id")?,
            channel_role: row.try_get("channel_role")?,
            workspace_role: row.try_get("workspace_role")?,
            dm_member: row.try_get("dm_member")?,
            project_member: row.try_get("project_member")?,
            channel_readable: row.try_get("channel_readable")?,
            team_archived: row.try_get("team_archived")?,
        })
    }
}
