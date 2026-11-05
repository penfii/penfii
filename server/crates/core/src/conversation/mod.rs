//! Channels, direct messages, and conversation metadata.
//! Message commands live in `messages`. Read cursors live in `reads`.

mod messages;
mod reads;
pub(crate) mod richtext;

pub use messages::{
    DeleteMessage, EditMessage, ReactionCommand, SendMessage, add_reaction, delete_message,
    edit_message, list_messages, list_thread, message_history, remove_reaction, send_message,
};
pub use reads::{
    ManualUnread, UpdateRead, get_conversation_read, get_thread_read, manual_conversation_unread,
    manual_thread_unread, update_conversation_read, update_thread_read,
};

use chrono::{DateTime, Utc};
use penfii_contracts::{
    Channel, ChannelMember, Conversation, ConversationChange, DirectMessage, ReadState,
};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::access::{self, ConvAccess};
use crate::error::CoreError;
use crate::identity::require_label;
use crate::platform::ChangeNotice;

#[derive(sqlx::FromRow)]
struct ChannelRow {
    id: Uuid,
    workspace_id: Uuid,
    name: String,
    description: Option<String>,
    topic: Option<String>,
    visibility: String,
    team_id: Option<Uuid>,
    project_id: Option<Uuid>,
    allow_member_invites: bool,
    archived_at: Option<DateTime<Utc>>,
    role: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub async fn create_channel(
    pool: &PgPool,
    actor_id: Uuid,
    workspace_id: Uuid,
    name: &str,
    description: Option<&str>,
    visibility: &str,
    team_id: Option<Uuid>,
    project_id: Option<Uuid>,
) -> Result<Channel, CoreError> {
    let name = require_label(name, "name")?;
    let description = match description.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) if value.chars().count() <= 2000 && !value.chars().any(char::is_control) => {
            Some(value.to_string())
        }
        Some(_) => return Err(CoreError::validation("description is invalid")),
        None => None,
    };
    if visibility != "PUBLIC" && visibility != "PRIVATE" {
        return Err(CoreError::validation(
            "visibility must be PUBLIC or PRIVATE",
        ));
    }
    // A Team narrows a Project's people; it is not a place to share on its own.
    if team_id.is_some() && project_id.is_none() {
        return Err(CoreError::validation(
            "a Team needs the Project it narrows (projectId)",
        ));
    }
    let mut tx = pool.begin().await?;
    let role = access::require_member(&mut tx, workspace_id, actor_id).await?;
    let workspace_admin = role == "OWNER" || role == "ADMIN";
    if let Some(team_id) = team_id {
        crate::admin::team_open_for_writes(&mut tx, workspace_id, team_id).await?;
    }
    if let Some(project_id) = project_id {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM penfii.projects WHERE id = $1 AND workspace_id = $2)",
        )
        .bind(project_id)
        .bind(workspace_id)
        .fetch_one(&mut *tx)
        .await?;
        if !exists {
            return Err(CoreError::NotFound);
        }
        if let Some(team_id) = team_id {
            let linked: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM penfii.project_teams WHERE project_id = $1 AND team_id = $2)",
            )
            .bind(project_id)
            .bind(team_id)
            .fetch_one(&mut *tx)
            .await?;
            if !linked {
                return Err(CoreError::validation(
                    "the Team is not one of the Project's Teams",
                ));
            }
        }
        // A Workspace admin has no way into a Project they do not take part in: its channels are made by its own admins.
        let (_, project_role) =
            crate::project_role::project_role(&mut tx, project_id, actor_id).await?;
        if !project_role
            .as_deref()
            .is_some_and(crate::project_role::can_manage)
        {
            return Err(CoreError::forbidden("project admin required"));
        }
    } else if !workspace_admin {
        return Err(CoreError::forbidden("workspace owner or admin required"));
    }
    let channel = insert_channel(
        &mut tx,
        actor_id,
        workspace_id,
        &name,
        description.as_deref(),
        visibility,
        team_id,
        project_id,
    )
    .await?;
    tx.commit().await?;
    Ok(channel)
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn insert_channel(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    actor_id: Uuid,
    workspace_id: Uuid,
    name: &str,
    description: Option<&str>,
    visibility: &str,
    team_id: Option<Uuid>,
    project_id: Option<Uuid>,
) -> Result<Channel, CoreError> {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO penfii.conversations (id, workspace_id, kind) VALUES ($1, $2, 'CHANNEL')",
    )
    .bind(id)
    .bind(workspace_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "INSERT INTO penfii.channels (id, team_id, project_id, name, visibility, description)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(id)
    .bind(team_id)
    .bind(project_id)
    .bind(name)
    .bind(visibility)
    .bind(description)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "INSERT INTO penfii.channel_members (channel_id, user_id, role)
         VALUES ($1, $2, 'MANAGER')",
    )
    .bind(id)
    .bind(actor_id)
    .execute(&mut **tx)
    .await?;
    crate::events::emit_simple(
        tx,
        workspace_id,
        "CHANNEL",
        id,
        "channel.created",
        Some(actor_id),
        serde_json::json!({"projectId": project_id, "teamId": team_id}),
    )
    .await?;
    Ok(Channel {
        id,
        workspace_id,
        name: name.to_string(),
        description: description.map(str::to_string),
        topic: None,
        visibility: visibility.to_string(),
        team_id,
        project_id,
        allow_member_invites: false,
        archived_at: None,
        role: Some("MANAGER".to_string()),
        unread_count: 0,
    })
}

pub async fn update_channel(
    pool: &PgPool,
    actor_id: Uuid,
    channel_id: Uuid,
    visibility: Option<&str>,
    description: Option<&str>,
) -> Result<Channel, CoreError> {
    if visibility.is_none() && description.is_none() {
        return Err(CoreError::validation(
            "visibility or description is required",
        ));
    }
    if let Some(visibility) = visibility
        && visibility != "PUBLIC"
        && visibility != "PRIVATE"
    {
        return Err(CoreError::validation(
            "visibility must be PUBLIC or PRIVATE",
        ));
    }
    let description = match description.map(str::trim) {
        Some("") => Some(None),
        Some(value) if value.chars().count() <= 2000 && !value.chars().any(char::is_control) => {
            Some(Some(value.to_string()))
        }
        Some(_) => return Err(CoreError::validation("description is invalid")),
        None => None,
    };
    let mut tx = pool.begin().await?;
    let Some(row) = load_channel(&mut tx, channel_id, actor_id).await? else {
        return Err(CoreError::NotFound);
    };
    let workspace_role = access::require_member(&mut tx, row.workspace_id, actor_id).await?;
    let project_admin = match row.project_id {
        Some(project_id) => crate::project_role::project_role(&mut tx, project_id, actor_id)
            .await?
            .1
            .as_deref()
            .is_some_and(crate::project_role::can_manage),
        None => false,
    };
    let workspace_admin = workspace_role == "OWNER" || workspace_role == "ADMIN";
    let manager = row.role.as_deref() == Some("MANAGER");
    let global_public = row.project_id.is_none() && row.visibility != "PRIVATE" && workspace_admin;
    if !manager && !project_admin && !global_public {
        if access::require_read(&mut tx, actor_id, channel_id)
            .await
            .is_err()
        {
            return Err(CoreError::NotFound);
        }
        return Err(CoreError::forbidden("channel manager required"));
    }
    if let Some(visibility) = visibility {
        sqlx::query("UPDATE penfii.channels SET visibility = $2 WHERE id = $1")
            .bind(channel_id)
            .bind(visibility)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(description) = description {
        sqlx::query("UPDATE penfii.channels SET description = $2 WHERE id = $1")
            .bind(channel_id)
            .bind(description)
            .execute(&mut *tx)
            .await?;
    }
    let channel = load_channel(&mut tx, channel_id, actor_id)
        .await?
        .map(channel_from)
        .ok_or(CoreError::NotFound)?;
    crate::events::emit_simple(
        &mut tx,
        channel.workspace_id,
        "CHANNEL",
        channel_id,
        if visibility.is_some() {
            "channel.visibility_changed"
        } else {
            "channel.updated"
        },
        Some(actor_id),
        serde_json::json!({"visibility": channel.visibility}),
    )
    .await?;
    tx.commit().await?;
    Ok(channel)
}

pub async fn join_channel(
    pool: &PgPool,
    actor_id: Uuid,
    channel_id: Uuid,
) -> Result<Channel, CoreError> {
    let mut tx = pool.begin().await?;
    let Some(row) = load_channel(&mut tx, channel_id, actor_id).await? else {
        return Err(CoreError::NotFound);
    };
    access::require_member(&mut tx, row.workspace_id, actor_id).await?;
    if row.role.is_some() {
        tx.commit().await?;
        return Ok(channel_from(row));
    }
    if row.visibility != "PUBLIC" {
        return Err(CoreError::NotFound);
    }
    if let Some(project_id) = row.project_id
        && !access::in_share_scope(&mut tx, project_id, row.team_id, actor_id).await?
    {
        return Err(CoreError::NotFound);
    }
    sqlx::query(
        "INSERT INTO penfii.channel_members (channel_id, user_id, role)
         VALUES ($1, $2, 'MEMBER')
         ON CONFLICT DO NOTHING",
    )
    .bind(channel_id)
    .bind(actor_id)
    .execute(&mut *tx)
    .await?;
    let channel = load_channel(&mut tx, channel_id, actor_id)
        .await?
        .map(channel_from)
        .ok_or(CoreError::NotFound)?;
    tx.commit().await?;
    Ok(channel)
}

/// The people of a channel, for anyone who can read it. Managers come first.
pub async fn list_channel_members(
    pool: &PgPool,
    actor_id: Uuid,
    channel_id: Uuid,
) -> Result<Vec<ChannelMember>, CoreError> {
    let mut tx = pool.begin().await?;
    let Some(access) = access::load_conversation(&mut tx, actor_id, channel_id).await? else {
        return Err(CoreError::NotFound);
    };
    if access.kind != "CHANNEL" || !access.can_read() {
        return Err(CoreError::NotFound);
    }
    let rows = sqlx::query_as::<_, (Uuid, String, String, DateTime<Utc>)>(
        "SELECT cm.user_id, u.display_name, cm.role, cm.joined_at
         FROM penfii.channel_members cm
         JOIN penfii.users u ON u.id = cm.user_id
         JOIN penfii.workspace_members wm
           ON wm.workspace_id = $2 AND wm.user_id = cm.user_id AND wm.state = 'ACTIVE'
         WHERE cm.channel_id = $1
         ORDER BY (cm.role = 'MANAGER') DESC, cm.joined_at, u.display_name",
    )
    .bind(channel_id)
    .bind(access.workspace_id)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows
        .into_iter()
        .map(|(user_id, display_name, role, joined_at)| ChannelMember {
            user_id,
            display_name,
            role,
            joined_at,
        })
        .collect())
}

/// Puts a person into a channel as a MEMBER (the call is repeatable). The people who manage the
/// channel do it, the same as for changing its visibility: its MANAGER, the admin of its Project,
/// and any member when the channel allows member invitations. The person must be in the Workspace
/// and, in a Project's channel, someone the Project's people include; a channel never gives
/// access to what its Project does not.
pub async fn add_channel_member(
    pool: &PgPool,
    actor_id: Uuid,
    channel_id: Uuid,
    user_id: Uuid,
) -> Result<(ChannelMember, Option<ChangeNotice>), CoreError> {
    let mut tx = pool.begin().await?;
    let Some(row) = load_channel(&mut tx, channel_id, actor_id).await? else {
        return Err(CoreError::NotFound);
    };
    access::require_member(&mut tx, row.workspace_id, actor_id).await?;
    let Some(access) = access::load_conversation(&mut tx, actor_id, channel_id).await? else {
        return Err(CoreError::NotFound);
    };
    if !access.can_read() {
        return Err(CoreError::NotFound);
    }
    let manager = row.role.as_deref() == Some("MANAGER");
    let project_admin = match row.project_id {
        Some(project_id) => crate::project_role::project_role(&mut tx, project_id, actor_id)
            .await?
            .1
            .as_deref()
            .is_some_and(crate::project_role::can_manage),
        None => false,
    };
    let member_invites = row.allow_member_invites && row.role.is_some();
    if !manager && !project_admin && !member_invites {
        return Err(CoreError::forbidden("channel manager required"));
    }
    access.require_writable()?;
    let active: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM penfii.workspace_members
         WHERE workspace_id = $1 AND user_id = $2 AND state = 'ACTIVE')",
    )
    .bind(row.workspace_id)
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;
    if !active {
        return Err(CoreError::validation(
            "that person is not a member of the Workspace",
        ));
    }
    if let Some(project_id) = row.project_id
        && !access::in_share_scope(&mut tx, project_id, row.team_id, user_id).await?
    {
        return Err(CoreError::validation(
            "that person does not take part in the Project of this channel",
        ));
    }
    let added = sqlx::query(
        "INSERT INTO penfii.channel_members (channel_id, user_id, role)
         VALUES ($1, $2, 'MEMBER')
         ON CONFLICT DO NOTHING",
    )
    .bind(channel_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if added == 1 {
        crate::events::emit_simple(
            &mut tx,
            row.workspace_id,
            "CHANNEL",
            channel_id,
            "channel.member_added",
            Some(actor_id),
            serde_json::json!({"userId": user_id}),
        )
        .await?;
    }
    let (display_name, role, joined_at): (String, String, DateTime<Utc>) = sqlx::query_as(
        "SELECT u.display_name, cm.role, cm.joined_at
         FROM penfii.channel_members cm JOIN penfii.users u ON u.id = cm.user_id
         WHERE cm.channel_id = $1 AND cm.user_id = $2",
    )
    .bind(channel_id)
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    // Only a person who was not in the channel has a new sidebar to read.
    let change = (added == 1).then_some(ChangeNotice::Space { user_id });
    Ok((
        ChannelMember {
            user_id,
            display_name,
            role,
            joined_at,
        },
        change,
    ))
}

/// Whether the channel keeps a manager (a person active in the Workspace) once `leaving` is no
/// longer one. The channel's managers are locked first, in a fixed order, so two people who each
/// give up the role at once (leaving, or being made members) cannot leave the channel without one.
async fn other_manager_stays(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    channel_id: Uuid,
    leaving: Uuid,
) -> Result<bool, CoreError> {
    sqlx::query(
        "SELECT user_id FROM penfii.channel_members
         WHERE channel_id = $1 AND role = 'MANAGER' ORDER BY user_id FOR UPDATE",
    )
    .bind(channel_id)
    .fetch_all(&mut **tx)
    .await?;
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM penfii.channel_members cm
                        JOIN penfii.conversations c ON c.id = cm.channel_id
                        JOIN penfii.workspace_members wm
                          ON wm.workspace_id = c.workspace_id AND wm.user_id = cm.user_id AND wm.state = 'ACTIVE'
                        WHERE cm.channel_id = $1 AND cm.role = 'MANAGER' AND cm.user_id <> $2)",
    )
    .bind(channel_id)
    .bind(leaving)
    .fetch_one(&mut **tx)
    .await?)
}

/// Makes a member of a private channel its MANAGER or a plain MEMBER again. Who may is who may put
/// people in without the member-invitation switch: the channel's MANAGER and the admin of its
/// Project. The last manager cannot be made a member, so the channel always has someone who runs it.
/// Giving a person the role they already have changes nothing and tells nobody.
pub async fn set_channel_member_role(
    pool: &PgPool,
    actor_id: Uuid,
    channel_id: Uuid,
    user_id: Uuid,
    role: &str,
) -> Result<(ChannelMember, Option<ChangeNotice>), CoreError> {
    if !matches!(role, "MANAGER" | "MEMBER") {
        return Err(CoreError::validation("role must be MANAGER or MEMBER"));
    }
    let mut tx = pool.begin().await?;
    let Some(row) = load_channel(&mut tx, channel_id, actor_id).await? else {
        return Err(CoreError::NotFound);
    };
    access::require_member(&mut tx, row.workspace_id, actor_id).await?;
    let Some(access) = access::load_conversation(&mut tx, actor_id, channel_id).await? else {
        return Err(CoreError::NotFound);
    };
    if !access.can_read() {
        return Err(CoreError::NotFound);
    }
    let manager = row.role.as_deref() == Some("MANAGER");
    let project_admin = match row.project_id {
        Some(project_id) => crate::project_role::project_role(&mut tx, project_id, actor_id)
            .await?
            .1
            .as_deref()
            .is_some_and(crate::project_role::can_manage),
        None => false,
    };
    if !manager && !project_admin {
        return Err(CoreError::forbidden("channel manager required"));
    }
    if row.visibility != "PRIVATE" {
        return Err(CoreError::validation(
            "roles are set in a private channel only",
        ));
    }
    access.require_writable()?;
    let current: Option<(String, String, DateTime<Utc>)> = sqlx::query_as(
        "SELECT cm.role, u.display_name, cm.joined_at
         FROM penfii.channel_members cm
         JOIN penfii.users u ON u.id = cm.user_id
         JOIN penfii.workspace_members wm
           ON wm.workspace_id = $3 AND wm.user_id = cm.user_id AND wm.state = 'ACTIVE'
         WHERE cm.channel_id = $1 AND cm.user_id = $2",
    )
    .bind(channel_id)
    .bind(user_id)
    .bind(row.workspace_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((before, display_name, joined_at)) = current else {
        return Err(CoreError::NotFound);
    };
    let member = |role: &str| ChannelMember {
        user_id,
        display_name: display_name.clone(),
        role: role.to_string(),
        joined_at,
    };
    if before == role {
        return Ok((member(role), None));
    }
    if before == "MANAGER" && !other_manager_stays(&mut tx, channel_id, user_id).await? {
        return Err(CoreError::conflict(
            "last_manager",
            "a channel needs at least one manager; make someone else a manager first",
        ));
    }
    let changed = sqlx::query(
        "UPDATE penfii.channel_members SET role = $3 WHERE channel_id = $1 AND user_id = $2",
    )
    .bind(channel_id)
    .bind(user_id)
    .bind(role)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if changed == 0 {
        // The person left between the read and the write.
        return Err(CoreError::NotFound);
    }
    crate::events::emit_simple(
        &mut tx,
        row.workspace_id,
        "CHANNEL",
        channel_id,
        "channel.member_role_changed",
        Some(actor_id),
        serde_json::json!({"userId": user_id, "role": role, "was": before}),
    )
    .await?;
    tx.commit().await?;
    Ok((member(role), Some(ChangeNotice::Space { user_id })))
}

/// A person leaves a private channel; nobody else can be taken out. The last MANAGER stays, so a
/// channel is never left without someone who can run it. Public channels are not left here.
/// Returns what the leaver's open screens must hear: their channel list changed and the sockets
/// they hold on the channel close.
pub async fn leave_channel(
    pool: &PgPool,
    actor_id: Uuid,
    channel_id: Uuid,
    user_id: Uuid,
) -> Result<Vec<ChangeNotice>, CoreError> {
    let mut tx = pool.begin().await?;
    let Some(row) = load_channel(&mut tx, channel_id, actor_id).await? else {
        return Err(CoreError::NotFound);
    };
    access::require_member(&mut tx, row.workspace_id, actor_id).await?;
    let Some(role) = row.role.as_deref() else {
        return Err(CoreError::NotFound);
    };
    if user_id != actor_id {
        return Err(CoreError::forbidden("you can only remove yourself"));
    }
    if row.visibility != "PRIVATE" {
        return Err(CoreError::validation("only a private channel can be left"));
    }
    if role == "MANAGER" && !other_manager_stays(&mut tx, channel_id, actor_id).await? {
        return Err(CoreError::conflict(
            "last_manager",
            "the last manager cannot leave; make someone else a manager first",
        ));
    }
    let removed =
        sqlx::query("DELETE FROM penfii.channel_members WHERE channel_id = $1 AND user_id = $2")
            .bind(channel_id)
            .bind(actor_id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
    if removed == 1 {
        crate::events::emit_simple(
            &mut tx,
            row.workspace_id,
            "CHANNEL",
            channel_id,
            "channel.member_removed",
            Some(actor_id),
            serde_json::json!({"userId": actor_id, "self": true}),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(vec![
        ChangeNotice::Space { user_id: actor_id },
        ChangeNotice::Access { user_id: actor_id },
    ])
}

pub async fn create_dm(
    pool: &PgPool,
    actor_id: Uuid,
    workspace_id: Uuid,
    user_ids: &[Uuid],
) -> Result<DirectMessage, CoreError> {
    let mut distinct = user_ids.to_vec();
    distinct.sort();
    distinct.dedup();
    if distinct.len() != user_ids.len() {
        return Err(CoreError::validation("userIds must be distinct"));
    }
    if distinct.len() < 2 {
        return Err(CoreError::validation(
            "userIds must include at least 2 distinct users",
        ));
    }
    if !distinct.contains(&actor_id) {
        return Err(CoreError::validation("userIds must include the caller"));
    }
    let mut tx = pool.begin().await?;
    access::require_member(&mut tx, workspace_id, actor_id).await?;
    let active: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.workspace_members
         WHERE workspace_id = $1 AND state = 'ACTIVE' AND user_id = ANY($2)",
    )
    .bind(workspace_id)
    .bind(&distinct)
    .fetch_one(&mut *tx)
    .await?;
    if active != i64::try_from(distinct.len()).unwrap_or(i64::MAX) {
        return Err(CoreError::validation(
            "userIds must all be active workspace members",
        ));
    }
    let member_count = i64::try_from(distinct.len()).unwrap_or(i64::MAX);
    let existing: Option<(Uuid, DateTime<Utc>, i64, i64)> =
        sqlx::query_as::<_, (Uuid, DateTime<Utc>, i64, i64)>(
            "SELECT c.id, c.created_at, c.last_message_seq, c.last_change_seq
         FROM penfii.conversations c
         WHERE c.workspace_id = $1
           AND c.kind = 'DM'
           AND (SELECT count(*) FROM penfii.dm_members m WHERE m.conversation_id = c.id) = $2
           AND (
             SELECT count(*) FROM penfii.dm_members m
             WHERE m.conversation_id = c.id AND m.user_id = ANY($3)
           ) = $2
         ORDER BY c.created_at, c.id
         LIMIT 1",
        )
        .bind(workspace_id)
        .bind(member_count)
        .bind(&distinct)
        .fetch_optional(&mut *tx)
        .await?;
    if let Some((id, created_at, last_message_seq, last_change_seq)) = existing {
        tx.commit().await?;
        return Ok(DirectMessage {
            id,
            workspace_id,
            user_ids: distinct,
            created_at,
            last_message_seq,
            last_change_seq,
            peers: Vec::new(),
            last_message_preview: None,
            last_message_at: None,
            unread_count: 0,
        });
    }
    let id = Uuid::new_v4();
    let created_at: DateTime<Utc> = sqlx::query_scalar(
        "INSERT INTO penfii.conversations (id, workspace_id, kind)
         VALUES ($1, $2, 'DM')
         RETURNING created_at",
    )
    .bind(id)
    .bind(workspace_id)
    .fetch_one(&mut *tx)
    .await?;
    for user_id in &distinct {
        sqlx::query("INSERT INTO penfii.dm_members (conversation_id, user_id) VALUES ($1, $2)")
            .bind(id)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(DirectMessage {
        id,
        workspace_id,
        user_ids: distinct,
        created_at,
        last_message_seq: 0,
        last_change_seq: 0,
        peers: Vec::new(),
        last_message_preview: None,
        last_message_at: None,
        unread_count: 0,
    })
}

pub async fn get_conversation(
    pool: &PgPool,
    user_id: Uuid,
    conversation_id: Uuid,
) -> Result<Conversation, CoreError> {
    let mut tx = pool.begin().await?;
    let access = access::require_read(&mut tx, user_id, conversation_id).await?;
    let channel = if access.kind == "CHANNEL" {
        load_channel(&mut tx, conversation_id, user_id)
            .await?
            .map(channel_from)
    } else {
        None
    };
    let member_ids = if access.kind == "DM" {
        sqlx::query_scalar(
            "SELECT user_id FROM penfii.dm_members WHERE conversation_id = $1 ORDER BY user_id",
        )
        .bind(conversation_id)
        .fetch_all(&mut *tx)
        .await?
    } else {
        Vec::new()
    };
    let read = reads::ensure_conversation_read(&mut tx, conversation_id, user_id).await?;
    let conversation = conversation_from(access, channel, member_ids, read);
    tx.commit().await?;
    Ok(conversation)
}

pub async fn readable_head(
    pool: &PgPool,
    user_id: Uuid,
    conversation_id: Uuid,
) -> Result<ConvAccess, CoreError> {
    let mut tx = pool.begin().await?;
    let access = access::require_read(&mut tx, user_id, conversation_id).await?;
    tx.commit().await?;
    Ok(access)
}

pub async fn list_changes(
    pool: &PgPool,
    user_id: Uuid,
    conversation_id: Uuid,
    after_seq: Option<i64>,
    limit: Option<i64>,
) -> Result<Vec<ConversationChange>, CoreError> {
    let limit = crate::paging::require_limit(limit, 50, 100, "limit must be between 1 and 100")?;
    let after_seq = after_seq.unwrap_or(0);
    if after_seq < 0 {
        return Err(CoreError::validation("afterSeq is invalid"));
    }
    let mut tx = pool.begin().await?;
    access::require_read(&mut tx, user_id, conversation_id).await?;
    let changes = sqlx::query_as::<_, ChangeRow>(
        "SELECT conversation_id, change_seq, message_id, message_version, change_kind, occurred_at
         FROM penfii.conversation_changes
         WHERE conversation_id = $1 AND change_seq > $2
         ORDER BY change_seq ASC
         LIMIT $3",
    )
    .bind(conversation_id)
    .bind(after_seq)
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(changes.into_iter().map(change_from).collect())
}

#[derive(sqlx::FromRow)]
struct ChangeRow {
    conversation_id: Uuid,
    change_seq: i64,
    message_id: Uuid,
    message_version: Option<i64>,
    change_kind: String,
    occurred_at: DateTime<Utc>,
}

fn change_from(row: ChangeRow) -> ConversationChange {
    ConversationChange {
        conversation_id: row.conversation_id,
        change_seq: row.change_seq,
        message_id: row.message_id,
        message_version: row.message_version,
        change_kind: row.change_kind,
        occurred_at: row.occurred_at,
    }
}

pub(crate) async fn require_thread_root(
    conn: &mut PgConnection,
    user_id: Uuid,
    root_id: Uuid,
) -> Result<ConvAccess, CoreError> {
    let row = sqlx::query_as::<_, (Uuid, Option<Uuid>)>(
        "SELECT conversation_id, thread_root_id FROM penfii.messages WHERE id = $1",
    )
    .bind(root_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((conversation_id, thread_root)) = row else {
        return Err(CoreError::NotFound);
    };
    if thread_root.is_some() {
        return Err(CoreError::NotFound);
    }
    access::require_read(conn, user_id, conversation_id).await
}

async fn load_channel(
    conn: &mut PgConnection,
    channel_id: Uuid,
    user_id: Uuid,
) -> Result<Option<ChannelRow>, CoreError> {
    let row = sqlx::query_as::<_, ChannelRow>(
        "SELECT ch.id, c.workspace_id, ch.name, ch.description, ch.topic, ch.visibility,
                ch.team_id, ch.project_id, ch.allow_member_invites, ch.archived_at, cm.role
         FROM penfii.channels ch
         JOIN penfii.conversations c ON c.id = ch.id
         LEFT JOIN penfii.channel_members cm
            ON cm.channel_id = ch.id AND cm.user_id = $2
         WHERE ch.id = $1",
    )
    .bind(channel_id)
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row)
}

fn channel_from(row: ChannelRow) -> Channel {
    Channel {
        id: row.id,
        workspace_id: row.workspace_id,
        name: row.name,
        description: row.description,
        topic: row.topic,
        visibility: row.visibility,
        team_id: row.team_id,
        project_id: row.project_id,
        allow_member_invites: row.allow_member_invites,
        archived_at: row.archived_at,
        role: row.role,
        unread_count: 0,
    }
}

fn conversation_from(
    access: ConvAccess,
    channel: Option<Channel>,
    member_ids: Vec<Uuid>,
    read: ReadState,
) -> Conversation {
    Conversation {
        id: access.id,
        workspace_id: access.workspace_id,
        kind: access.kind,
        last_message_seq: access.last_message_seq,
        last_change_seq: access.last_change_seq,
        replay_floor_seq: access.replay_floor_seq,
        created_at: access.created_at,
        channel,
        member_ids,
        read,
    }
}
