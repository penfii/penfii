//! The managers of private channels when people leave: what leaving costs, and the way back.
//!
//! * A channel's "active manager" is a `MANAGER` row whose person is an `ACTIVE` member of the
//!   Workspace and whose account is not disabled ([`ACTIVE_CHANNEL_MANAGERS`], the same meaning of
//!   "active" as the Project admins of `project_access`) and who, in a Project's channel, still takes
//!   part in the Project: a Manager row left by a participation that is gone is no manager. The guard against the last manager leaving
//!   or being demoted, the warning before an offboarding and the recovery below all read it.
//! * Removing a person from the Workspace takes all of their channel memberships, private and public
//!   ([`clear_memberships`]). A person who comes back is in no channel; a private one has to be
//!   given to them again. What they read in a public channel is decided by the channel's own policy.
//! * A private channel left with no active manager is recovered by a Workspace owner or admin who
//!   names a manager ([`recover_manager`]); how it got there does not matter. A public channel
//!   without a manager is an ordinary state and is not recovered here.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::access;
use crate::error::CoreError;
use crate::platform::ChangeNotice;

/// `(channel_id, user_id)` for every active manager, as a subquery.
pub(crate) const ACTIVE_CHANNEL_MANAGERS: &str = "(SELECT cm.channel_id, cm.user_id
        FROM penfii.channel_members cm
        JOIN penfii.conversations c ON c.id = cm.channel_id
        JOIN penfii.channels ch ON ch.id = cm.channel_id
        JOIN penfii.workspace_members wm
          ON wm.workspace_id = c.workspace_id AND wm.user_id = cm.user_id AND wm.state = 'ACTIVE'
        JOIN penfii.users u ON u.id = cm.user_id AND u.disabled_at IS NULL
        WHERE cm.role = 'MANAGER'
          AND (ch.project_id IS NULL OR penfii.in_share_scope(ch.project_id, ch.team_id, cm.user_id)))";

/// A private channel that would be left without an active manager if the person left.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelImpact {
    pub channel_id: Uuid,
    pub name: String,
    /// The Project the channel belongs to; none for a Workspace-wide channel.
    pub project_id: Option<Uuid>,
}

/// The private channels the person is the last active manager of. For the warning before removing a
/// member or disabling an account and for the operator's `user-disable`; it only informs, nothing
/// is refused because of it. Archived channels and public channels are left out.
pub(crate) async fn offboarding_impact_in(
    conn: &mut PgConnection,
    workspace_id: Uuid,
    user_id: Uuid,
) -> Result<Vec<ChannelImpact>, CoreError> {
    let rows = sqlx::query_as::<_, (Uuid, String, Option<Uuid>)>(&format!(
        "SELECT ch.id, ch.name, ch.project_id
         FROM penfii.channels ch JOIN penfii.conversations c ON c.id = ch.id
         WHERE c.workspace_id = $1 AND ch.visibility = 'PRIVATE' AND ch.archived_at IS NULL
           AND EXISTS (SELECT 1 FROM {m} x WHERE x.channel_id = ch.id AND x.user_id = $2)
           AND NOT EXISTS (SELECT 1 FROM {m} x WHERE x.channel_id = ch.id AND x.user_id <> $2)
         ORDER BY ch.name, ch.id",
        m = ACTIVE_CHANNEL_MANAGERS
    ))
    .bind(workspace_id)
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(channel_id, name, project_id)| ChannelImpact {
            channel_id,
            name,
            project_id,
        })
        .collect())
}

/// `GET .../members/{user_id}/offboarding-impact`, the channel half. Workspace owners and admins
/// only; the person must be an active member.
pub async fn offboarding_impact(
    pool: &PgPool,
    actor: Uuid,
    workspace_id: Uuid,
    user_id: Uuid,
) -> Result<Vec<ChannelImpact>, CoreError> {
    let mut conn = pool.acquire().await?;
    crate::admin::require_manager(&mut conn, workspace_id, actor).await?;
    let member: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM penfii.workspace_members
                        WHERE workspace_id = $1 AND user_id = $2 AND state = 'ACTIVE')",
    )
    .bind(workspace_id)
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    if !member {
        return Err(CoreError::NotFound);
    }
    offboarding_impact_in(&mut conn, workspace_id, user_id).await
}

/// A channel membership that [`clear_memberships`] took away.
#[derive(Debug, Clone)]
pub(crate) struct ClearedMembership {
    pub channel_id: Uuid,
    pub private: bool,
}

/// Takes the person out of every channel of the Workspace, private and public. Their messages and
/// everything that is kept per person (read positions, mutes, favorites, later items, notification
/// settings) stay: those tables do not hang on the membership. The rows are locked in a fixed
/// order, the same as the manager guards lock theirs.
pub(crate) async fn clear_memberships(
    conn: &mut PgConnection,
    workspace_id: Uuid,
    user_id: Uuid,
) -> Result<Vec<ClearedMembership>, CoreError> {
    let rows = sqlx::query_as::<_, (Uuid, String)>(
        "SELECT cm.channel_id, ch.visibility
         FROM penfii.channel_members cm
         JOIN penfii.channels ch ON ch.id = cm.channel_id
         JOIN penfii.conversations c ON c.id = cm.channel_id
         WHERE c.workspace_id = $1 AND cm.user_id = $2
         ORDER BY cm.channel_id FOR UPDATE OF cm",
    )
    .bind(workspace_id)
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<Uuid> = rows.iter().map(|(id, _)| *id).collect();
    sqlx::query("DELETE FROM penfii.channel_members WHERE user_id = $1 AND channel_id = ANY($2)")
        .bind(user_id)
        .bind(&ids)
        .execute(&mut *conn)
        .await?;
    Ok(rows
        .into_iter()
        .map(|(channel_id, visibility)| ClearedMembership {
            channel_id,
            private: visibility == "PRIVATE",
        })
        .collect())
}

/// Serialises the changes of who takes part in a Project with the additions of channel members to
/// its channels. A participation change takes it after changing the participation and before
/// [`clear_lost_participation`]; an addition takes it before checking that the person takes part.
/// Whichever comes first, no row is left behind for a person who does not take part.
pub(crate) async fn lock_project_channels(
    conn: &mut PgConnection,
    project_id: Uuid,
) -> Result<(), CoreError> {
    sqlx::query(
        "SELECT pg_advisory_xact_lock(hashtextextended('project-channels:' || $1::text, 0))",
    )
    .bind(project_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// What [`clear_lost_participation`] took away.
#[derive(Debug, Clone, Default)]
pub(crate) struct LostParticipation {
    /// `(channel_id, user_id, private)` for every membership removed.
    pub cleared: Vec<(Uuid, Uuid, bool)>,
    /// Private channels among them that now have no active manager (recoverable, FEA-051).
    pub without_manager: Vec<Uuid>,
}

impl LostParticipation {
    /// The counts for the audit entry of the change that caused it.
    pub fn audit_data(&self) -> serde_json::Value {
        let private = self
            .cleared
            .iter()
            .filter(|(_, _, private)| *private)
            .count();
        serde_json::json!({
            "clearedChannels": {"private": private, "public": self.cleared.len() - private},
            "channelsWithoutManager": self.without_manager,
        })
    }

    /// `base` (an audit entry's own facts, an object) with the counts added.
    pub fn audit_with(&self, mut base: serde_json::Value) -> serde_json::Value {
        if let (Some(target), serde_json::Value::Object(extra)) =
            (base.as_object_mut(), self.audit_data())
        {
            target.extend(extra);
        }
        base
    }

    /// People whose open channels and Space list have to be read again.
    pub fn notices(&self) -> Vec<ChangeNotice> {
        let mut users: Vec<Uuid> = self.cleared.iter().map(|(_, user, _)| *user).collect();
        users.sort();
        users.dedup();
        users
            .into_iter()
            .flat_map(|user_id| {
                [
                    ChangeNotice::Access { user_id },
                    ChangeNotice::Space { user_id },
                ]
            })
            .collect()
    }
}

/// Takes the memberships (and with them the manager role) of Project channels away from people who
/// no longer take part in the Project, public and private channels, Team-narrowed ones included.
/// "No longer" is decided now, after the change that was made: a person who still takes part through
/// another way (a direct role, a Team, a Group) keeps theirs. `project` limits it to one Project,
/// `user` to one person; either may be left open. Coming back to the Project brings none of it back.
/// Messages and the per-person settings (read position, mutes, favorites, later items, notification
/// settings) stay. Each membership is announced as `channel.member_removed`.
pub(crate) async fn clear_lost_participation(
    conn: &mut PgConnection,
    workspace_id: Uuid,
    project: Option<Uuid>,
    user: Option<Uuid>,
    actor: Uuid,
) -> Result<LostParticipation, CoreError> {
    let rows = sqlx::query_as::<_, (Uuid, Uuid, String)>(
        "SELECT cm.channel_id, cm.user_id, ch.visibility
         FROM penfii.channel_members cm
         JOIN penfii.channels ch ON ch.id = cm.channel_id
         JOIN penfii.conversations c ON c.id = cm.channel_id
         WHERE c.workspace_id = $1 AND ch.project_id IS NOT NULL
           AND ($2::uuid IS NULL OR ch.project_id = $2)
           AND ($3::uuid IS NULL OR cm.user_id = $3)
           AND NOT penfii.in_share_scope(ch.project_id, ch.team_id, cm.user_id)
         ORDER BY cm.channel_id, cm.user_id FOR UPDATE OF cm",
    )
    .bind(workspace_id)
    .bind(project)
    .bind(user)
    .fetch_all(&mut *conn)
    .await?;
    if rows.is_empty() {
        return Ok(LostParticipation::default());
    }
    let channels: Vec<Uuid> = rows.iter().map(|(channel, _, _)| *channel).collect();
    let users: Vec<Uuid> = rows.iter().map(|(_, user, _)| *user).collect();
    sqlx::query(
        "DELETE FROM penfii.channel_members cm
         USING unnest($1::uuid[], $2::uuid[]) AS gone(channel_id, user_id)
         WHERE cm.channel_id = gone.channel_id AND cm.user_id = gone.user_id",
    )
    .bind(&channels)
    .bind(&users)
    .execute(&mut *conn)
    .await?;
    let mut cleared = Vec::with_capacity(rows.len());
    for (channel_id, user_id, visibility) in rows {
        crate::events::emit_simple(
            &mut *conn,
            workspace_id,
            "CHANNEL",
            channel_id,
            "channel.member_removed",
            Some(actor),
            serde_json::json!({"userId": user_id, "reason": "project_participation_lost"}),
        )
        .await?;
        cleared.push((channel_id, user_id, visibility == "PRIVATE"));
    }
    let private: Vec<Uuid> = {
        let mut ids: Vec<Uuid> = cleared
            .iter()
            .filter(|(_, _, private)| *private)
            .map(|(channel, _, _)| *channel)
            .collect();
        ids.sort();
        ids.dedup();
        ids
    };
    let without_manager: Vec<Uuid> = sqlx::query_scalar(&format!(
        "SELECT ch.id FROM penfii.channels ch
         WHERE ch.id = ANY($1) AND NOT EXISTS (SELECT 1 FROM {m} x WHERE x.channel_id = ch.id)
         ORDER BY ch.id",
        m = ACTIVE_CHANNEL_MANAGERS
    ))
    .bind(&private)
    .fetch_all(&mut *conn)
    .await?;
    Ok(LostParticipation {
        cleared,
        without_manager,
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormerManager {
    pub user_id: Uuid,
    pub display_name: String,
}

/// A private channel nobody can manage right now, for a Workspace owner or admin who may name a manager.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelWithoutManager {
    pub id: Uuid,
    pub name: String,
    pub project_id: Option<Uuid>,
    pub project_name: Option<String>,
    /// People who can read the channel now (active members of the Workspace in it).
    pub member_count: i64,
    /// Who held the manager role and cannot act now (left the Workspace, or account disabled).
    pub former_managers: Vec<FormerManager>,
    pub created_at: DateTime<Utc>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct FormerManagerRow {
    user_id: Uuid,
    display_name: String,
}

type WithoutManagerRow = (
    Uuid,
    String,
    Option<Uuid>,
    Option<String>,
    i64,
    sqlx::types::Json<Vec<FormerManagerRow>>,
    DateTime<Utc>,
);

/// The private channels of the Workspace that have no active manager, whatever the cause (no
/// manager row at all, only removed or disabled managers, made private without a manager), oldest
/// first. Workspace owners and admins only; archived and public channels are left out. Only names
/// and counts are shown, never messages.
pub async fn channels_without_manager(
    pool: &PgPool,
    actor: Uuid,
    workspace_id: Uuid,
) -> Result<Vec<ChannelWithoutManager>, CoreError> {
    let mut conn = pool.acquire().await?;
    crate::admin::require_manager(&mut conn, workspace_id, actor).await?;
    let rows = sqlx::query_as::<_, WithoutManagerRow>(&format!(
        "SELECT ch.id, ch.name, ch.project_id, p.name,
                (SELECT count(*) FROM penfii.channel_members cm
                 JOIN penfii.workspace_members wm
                   ON wm.workspace_id = c.workspace_id AND wm.user_id = cm.user_id AND wm.state = 'ACTIVE'
                 WHERE cm.channel_id = ch.id
                   AND (ch.project_id IS NULL
                        OR penfii.in_share_scope(ch.project_id, ch.team_id, cm.user_id))),
                COALESCE((SELECT jsonb_agg(jsonb_build_object('userId', u.id, 'displayName', u.display_name)
                                           ORDER BY u.display_name, u.id)
                          FROM penfii.channel_members cm JOIN penfii.users u ON u.id = cm.user_id
                          WHERE cm.channel_id = ch.id AND cm.role = 'MANAGER'), '[]'::jsonb),
                c.created_at
         FROM penfii.channels ch
         JOIN penfii.conversations c ON c.id = ch.id
         LEFT JOIN penfii.projects p ON p.id = ch.project_id
         WHERE c.workspace_id = $1 AND ch.visibility = 'PRIVATE' AND ch.archived_at IS NULL
           AND NOT EXISTS (SELECT 1 FROM {m} x WHERE x.channel_id = ch.id)
         ORDER BY c.created_at, ch.id",
        m = ACTIVE_CHANNEL_MANAGERS
    ))
    .bind(workspace_id)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(id, name, project_id, project_name, member_count, former, created_at)| {
                ChannelWithoutManager {
                    id,
                    name,
                    project_id,
                    project_name,
                    member_count,
                    former_managers: former
                        .0
                        .into_iter()
                        .map(|f| FormerManager {
                            user_id: f.user_id,
                            display_name: f.display_name,
                        })
                        .collect(),
                    created_at,
                }
            },
        )
        .collect())
}

/// What a recovery did.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagerRecovery {
    /// The named person was not in the channel before, so this gave them access to its content.
    pub new_access: bool,
}

/// A Workspace owner or admin names a manager for a private channel that has no active manager. It
/// is the only thing this allows: it gives the person who names no powers in the channel (naming
/// themself is the one way they get any), nobody is promoted on their own, and a channel that has an
/// active manager, or a public one, is refused (`channel_has_manager`, `channel_not_private`). The
/// named person must be an active member of the Workspace whose account is not disabled, and in a
/// Project's channel one of the Project's people. The channel is locked, so two recoveries at once
/// name one. A person who was already in the channel keeps their place and becomes its manager.
/// Audited as `channel.manager_recovered`, saying whether this opened the channel to someone who
/// could not read it.
pub async fn recover_manager(
    pool: &PgPool,
    actor: Uuid,
    channel_id: Uuid,
    user_id: Uuid,
) -> Result<(ManagerRecovery, Vec<ChangeNotice>), CoreError> {
    let mut tx = pool.begin().await?;
    let channel: Option<(Uuid, String, Option<Uuid>, Option<Uuid>)> = sqlx::query_as(
        "SELECT c.workspace_id, ch.visibility, ch.project_id, ch.team_id
         FROM penfii.channels ch JOIN penfii.conversations c ON c.id = ch.id
         WHERE ch.id = $1 FOR UPDATE OF ch",
    )
    .bind(channel_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((workspace, visibility, project_id, team_id)) = channel else {
        return Err(CoreError::NotFound);
    };
    crate::admin::require_manager(&mut tx, workspace, actor).await?;
    if visibility != "PRIVATE" {
        return Err(CoreError::conflict(
            "channel_not_private",
            "only a private channel is recovered this way; a public channel can do without a manager",
        ));
    }
    // The managers are locked in a fixed order, like the guards of the manager role do.
    sqlx::query(
        "SELECT user_id FROM penfii.channel_members
         WHERE channel_id = $1 AND role = 'MANAGER' ORDER BY user_id FOR UPDATE",
    )
    .bind(channel_id)
    .fetch_all(&mut *tx)
    .await?;
    let has_manager: bool = sqlx::query_scalar(&format!(
        "SELECT EXISTS (SELECT 1 FROM {m} x WHERE x.channel_id = $1)",
        m = ACTIVE_CHANNEL_MANAGERS
    ))
    .bind(channel_id)
    .fetch_one(&mut *tx)
    .await?;
    if has_manager {
        return Err(CoreError::conflict(
            "channel_has_manager",
            "the channel has an active manager; only a channel without one is recovered this way",
        ));
    }
    let target: Option<bool> = sqlx::query_scalar(
        "SELECT u.disabled_at IS NOT NULL
         FROM penfii.workspace_members wm JOIN penfii.users u ON u.id = wm.user_id
         WHERE wm.workspace_id = $1 AND wm.user_id = $2 AND wm.state = 'ACTIVE'",
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
    if let Some(project_id) = project_id {
        lock_project_channels(&mut tx, project_id).await?;
    }
    if let Some(project_id) = project_id
        && !access::in_share_scope(&mut tx, project_id, team_id, user_id).await?
    {
        return Err(CoreError::rule(
            "not_in_project",
            "that person does not take part in the Project of this channel",
        ));
    }
    let before: Option<String> = sqlx::query_scalar(
        "SELECT role FROM penfii.channel_members WHERE channel_id = $1 AND user_id = $2",
    )
    .bind(channel_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO penfii.channel_members (channel_id, user_id, role) VALUES ($1, $2, 'MANAGER')
         ON CONFLICT (channel_id, user_id) DO UPDATE SET role = 'MANAGER'",
    )
    .bind(channel_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    let new_access = before.is_none();
    crate::events::emit_simple(
        &mut tx,
        workspace,
        "CHANNEL",
        channel_id,
        "channel.manager_recovered",
        Some(actor),
        serde_json::json!({"userId": user_id, "self": actor == user_id, "newAccess": new_access}),
    )
    .await?;
    crate::admin::audit(
        &mut tx,
        workspace,
        Some(actor),
        "channel.manager_recovered",
        "CHANNEL",
        channel_id,
        serde_json::json!({
            "userId": user_id,
            "self": actor == user_id,
            "newAccess": new_access,
            "previousRole": before,
        }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        ManagerRecovery { new_access },
        vec![ChangeNotice::Space { user_id }],
    ))
}
