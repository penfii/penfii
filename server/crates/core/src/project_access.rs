//! Who is in a Project and how: direct grants to named people, and the Teams the Project is linked to.
//!
//! A Team link is an organizational relation and gives no one access. A link may carry a participation role;
//! the Team's current members then reach the Project with that role for as long as they are in the Team
//! (`penfii.in_share_scope`, `project_role::project_role`). Nothing here copies Team members into direct grants.

use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::admin::audit;
use crate::conversation::recovery::{clear_lost_participation, lock_project_channels};
use crate::error::CoreError;
use crate::platform::ChangeNotice;
use crate::project_role::{project_role, require_project_admin};

fn project_access_role(role: &str) -> Result<&str, CoreError> {
    match role {
        "ADMIN" | "EDITOR" | "VIEWER" => Ok(role),
        _ => Err(CoreError::validation(
            "role must be ADMIN, EDITOR or VIEWER",
        )),
    }
}

/// Locks the Project row and checks the caller administers it; returns its Workspace.
async fn admin_of(
    conn: &mut PgConnection,
    project_id: Uuid,
    actor: Uuid,
) -> Result<Uuid, CoreError> {
    require_project_admin(conn, project_id, actor).await?;
    let (workspace, _) = project_role(conn, project_id, actor).await?;
    sqlx::query("SELECT 1 FROM penfii.projects WHERE id = $1 FOR UPDATE")
        .bind(project_id)
        .fetch_one(&mut *conn)
        .await?;
    Ok(workspace)
}

/// Every way a person is a Project admin right now, as rows of `(project_id, user_id, source)`: a direct
/// grant, a Team that takes part with the admin role, or a Group (or the Team behind it) with the admin
/// role. Only people who can act are listed: an `ACTIVE` member of the Project's Workspace whose account
/// is not disabled. This is the one definition of "an active admin"; the rule that a Project keeps a
/// direct admin, the recovery of a Project without one and the offboarding warning all read it.
/// The legacy `OWNER` role counts as `ADMIN`, as in [`project_role`].
pub(crate) const ACTIVE_ADMIN_SOURCES: &str = "(
    SELECT s.project_id, s.user_id, s.source
    FROM (
        SELECT g.project_id, g.user_id, 'DIRECT' AS source
          FROM penfii.project_user_grants g WHERE g.role IN ('ADMIN', 'OWNER')
        UNION ALL
        SELECT pt.project_id, tm.user_id, 'TEAM'
          FROM penfii.project_teams pt JOIN penfii.team_members tm ON tm.team_id = pt.team_id
          WHERE pt.access_role IN ('ADMIN', 'OWNER')
        UNION ALL
        SELECT pg.project_id, gm.user_id, 'GROUP'
          FROM penfii.project_group_grants pg
          JOIN penfii.groups gr ON gr.id = pg.group_id AND gr.deleted_at IS NULL
          JOIN penfii.group_members gm ON gm.group_id = gr.id
          WHERE pg.role IN ('ADMIN', 'OWNER')
        UNION ALL
        SELECT pg.project_id, tm.user_id, 'GROUP'
          FROM penfii.project_group_grants pg
          JOIN penfii.groups gr ON gr.id = pg.group_id AND gr.deleted_at IS NULL
          JOIN penfii.team_members tm ON tm.team_id = gr.system_team_id
          WHERE pg.role IN ('ADMIN', 'OWNER')
    ) s
    JOIN penfii.projects p ON p.id = s.project_id
    JOIN penfii.workspace_members wm
      ON wm.workspace_id = p.workspace_id AND wm.user_id = s.user_id AND wm.state = 'ACTIVE'
    JOIN penfii.users u ON u.id = s.user_id AND u.disabled_at IS NULL
)";

/// Whether the Project has an active admin by any source, not counting `ignoring` (the person who is
/// about to leave, when asking what their leaving would do).
pub(crate) async fn has_active_admin(
    conn: &mut PgConnection,
    project_id: Uuid,
    ignoring: Option<Uuid>,
) -> Result<bool, CoreError> {
    Ok(sqlx::query_scalar(&format!(
        "SELECT EXISTS (SELECT 1 FROM {ACTIVE_ADMIN_SOURCES} a
                        WHERE a.project_id = $1 AND a.user_id IS DISTINCT FROM $2)"
    ))
    .bind(project_id)
    .bind(ignoring)
    .fetch_one(&mut *conn)
    .await?)
}

/// Whether the person is an active direct admin of the Project, and whether another one remains.
async fn direct_admin_standing(
    conn: &mut PgConnection,
    project_id: Uuid,
    user_id: Uuid,
) -> Result<(bool, bool), CoreError> {
    let (is_one, others): (bool, bool) = sqlx::query_as(&format!(
        "SELECT EXISTS (SELECT 1 FROM {ACTIVE_ADMIN_SOURCES} a
                        WHERE a.project_id = $1 AND a.source = 'DIRECT' AND a.user_id = $2),
                EXISTS (SELECT 1 FROM {ACTIVE_ADMIN_SOURCES} a
                        WHERE a.project_id = $1 AND a.source = 'DIRECT' AND a.user_id <> $2)"
    ))
    .bind(project_id)
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    Ok((is_one, others))
}

/// A Project keeps an active admin of its own: the last active direct admin cannot be demoted or have
/// their grant taken away (by anyone, themself included). Only an action that would remove that last
/// one is refused; with no active direct admin to begin with (they left the Workspace, or the
/// Project is run through a Team) other changes to grants are not held up by it. Leaving the
/// Workspace or disabling the account is not an action on the grant and is never refused here.
/// The caller has locked the Project (`admin_of`), so two changes at once queue.
async fn require_direct_admin_stays(
    conn: &mut PgConnection,
    project_id: Uuid,
    user_id: Uuid,
) -> Result<(), CoreError> {
    let (is_active_direct_admin, others) = direct_admin_standing(conn, project_id, user_id).await?;
    if is_active_direct_admin && !others {
        return Err(CoreError::conflict(
            "last_project_admin",
            "a Project keeps at least one active admin of its own; make someone else its admin first",
        ));
    }
    Ok(())
}

/// Gives a Workspace member a direct role in the Project, or changes the role they have.
pub async fn set_grant(
    pool: &PgPool,
    actor: Uuid,
    project_id: Uuid,
    user_id: Uuid,
    role: &str,
) -> Result<(), CoreError> {
    let role = project_access_role(role)?;
    let mut tx = pool.begin().await?;
    let workspace = admin_of(&mut tx, project_id, actor).await?;
    let active: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM penfii.workspace_members
                       WHERE workspace_id = $1 AND user_id = $2 AND state = 'ACTIVE')",
    )
    .bind(workspace)
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;
    if !active {
        return Err(CoreError::validation(
            "the person must be an active member of the Workspace",
        ));
    }
    if role != "ADMIN" {
        // Giving the role to a direct admin who is the last one would leave the Project without.
        require_direct_admin_stays(&mut tx, project_id, user_id).await?;
    }
    // Designations (waive dependencies, manage pins) belong to a direct editor only.
    sqlx::query(
        "INSERT INTO penfii.project_user_grants (project_id, user_id, role) VALUES ($1, $2, $3)
         ON CONFLICT (project_id, user_id) DO UPDATE SET role = EXCLUDED.role,
            can_waive_dependencies = project_user_grants.can_waive_dependencies AND EXCLUDED.role = 'EDITOR',
            can_manage_pins = project_user_grants.can_manage_pins AND EXCLUDED.role = 'EDITOR'",
    )
    .bind(project_id)
    .bind(user_id)
    .bind(role)
    .execute(&mut *tx)
    .await?;
    audit(
        &mut tx,
        workspace,
        Some(actor),
        "project.grant_set",
        "PROJECT",
        project_id,
        serde_json::json!({"userId": user_id, "role": role}),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Takes a direct role away. What the person reaches through a Team stays as it is.
pub async fn remove_grant(
    pool: &PgPool,
    actor: Uuid,
    project_id: Uuid,
    user_id: Uuid,
) -> Result<Vec<ChangeNotice>, CoreError> {
    let mut tx = pool.begin().await?;
    let workspace = admin_of(&mut tx, project_id, actor).await?;
    require_direct_admin_stays(&mut tx, project_id, user_id).await?;
    let removed = sqlx::query(
        "DELETE FROM penfii.project_user_grants WHERE project_id = $1 AND user_id = $2",
    )
    .bind(project_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if removed == 0 {
        return Err(CoreError::NotFound);
    }
    // What the person reaches through a Team stays; a channel membership is kept only while they
    // still take part in the Project some other way.
    lock_project_channels(&mut tx, project_id).await?;
    let lost = clear_lost_participation(&mut tx, workspace, Some(project_id), Some(user_id), actor)
        .await?;
    audit(
        &mut tx,
        workspace,
        Some(actor),
        "project.grant_removed",
        "PROJECT",
        project_id,
        lost.audit_with(serde_json::json!({"userId": user_id})),
    )
    .await?;
    tx.commit().await?;
    Ok(lost.notices())
}

async fn team_of(conn: &mut PgConnection, workspace: Uuid, team_id: Uuid) -> Result<(), CoreError> {
    let found: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM penfii.teams WHERE id = $1 AND workspace_id = $2 AND archived_at IS NULL)",
    )
    .bind(team_id)
    .bind(workspace)
    .fetch_one(&mut *conn)
    .await?;
    if found {
        Ok(())
    } else {
        Err(CoreError::NotFound)
    }
}

/// Links a Team to the Project. Linking again changes nothing; a link never gives access by itself.
pub async fn link_team(
    pool: &PgPool,
    actor: Uuid,
    project_id: Uuid,
    team_id: Uuid,
) -> Result<(), CoreError> {
    let mut tx = pool.begin().await?;
    let workspace = admin_of(&mut tx, project_id, actor).await?;
    team_of(&mut tx, workspace, team_id).await?;
    let added = sqlx::query(
        "INSERT INTO penfii.project_teams (project_id, team_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(project_id)
    .bind(team_id)
    .execute(&mut *tx)
    .await?
    .rows_affected()
        > 0;
    if added {
        audit(
            &mut tx,
            workspace,
            Some(actor),
            "project.team_linked",
            "PROJECT",
            project_id,
            serde_json::json!({"teamId": team_id}),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Removes the link, and with it the Team's participation.
pub async fn unlink_team(
    pool: &PgPool,
    actor: Uuid,
    project_id: Uuid,
    team_id: Uuid,
) -> Result<Vec<ChangeNotice>, CoreError> {
    let mut tx = pool.begin().await?;
    let workspace = admin_of(&mut tx, project_id, actor).await?;
    let removed =
        sqlx::query("DELETE FROM penfii.project_teams WHERE project_id = $1 AND team_id = $2")
            .bind(project_id)
            .bind(team_id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
    if removed == 0 {
        return Err(CoreError::NotFound);
    }
    lock_project_channels(&mut tx, project_id).await?;
    let lost = clear_lost_participation(&mut tx, workspace, Some(project_id), None, actor).await?;
    audit(
        &mut tx,
        workspace,
        Some(actor),
        "project.team_unlinked",
        "PROJECT",
        project_id,
        lost.audit_with(serde_json::json!({"teamId": team_id})),
    )
    .await?;
    tx.commit().await?;
    Ok(lost.notices())
}

/// Lets the members of a linked Team into the Project with `role`, or (`None`) ends the participation and
/// keeps the link. A Team that is not linked yet is refused.
pub async fn set_team_participation(
    pool: &PgPool,
    actor: Uuid,
    project_id: Uuid,
    team_id: Uuid,
    role: Option<&str>,
) -> Result<Vec<ChangeNotice>, CoreError> {
    let role = role.map(project_access_role).transpose()?;
    let mut tx = pool.begin().await?;
    let workspace = admin_of(&mut tx, project_id, actor).await?;
    // An archived Team keeps its participation and may be lowered, changed or unlinked, but is not
    // given a role it did not have.
    let state: Option<(bool, Option<String>)> = sqlx::query_as(
        "SELECT t.archived_at IS NOT NULL, pt.access_role
         FROM penfii.project_teams pt JOIN penfii.teams t ON t.id = pt.team_id
         WHERE pt.project_id = $1 AND pt.team_id = $2 FOR UPDATE OF pt",
    )
    .bind(project_id)
    .bind(team_id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some((true, None)) = state
        && role.is_some()
    {
        return Err(CoreError::conflict(
            "team_archived",
            "an archived Team cannot be given a new role",
        ));
    }
    let updated = sqlx::query(
        "UPDATE penfii.project_teams SET access_role = $3 WHERE project_id = $1 AND team_id = $2",
    )
    .bind(project_id)
    .bind(team_id)
    .bind(role)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if updated == 0 {
        return Err(CoreError::validation(
            "link the Team to the Project before giving it a role",
        ));
    }
    // Only ending the participation can take someone out; a changed role keeps them in.
    let lost = if role.is_none() {
        lock_project_channels(&mut tx, project_id).await?;
        clear_lost_participation(&mut tx, workspace, Some(project_id), None, actor).await?
    } else {
        Default::default()
    };
    audit(
        &mut tx,
        workspace,
        Some(actor),
        "project.team_participation_set",
        "PROJECT",
        project_id,
        lost.audit_with(serde_json::json!({"teamId": team_id, "role": role})),
    )
    .await?;
    tx.commit().await?;
    Ok(lost.notices())
}

/// The active members of a Team linked to the Project, for a Project admin choosing people to give a direct
/// role. Only the names are shown, and only for a Team that is linked to this Project.
pub async fn team_roster(
    pool: &PgPool,
    actor: Uuid,
    project_id: Uuid,
    team_id: Uuid,
) -> Result<Vec<penfii_contracts::ProjectMember>, CoreError> {
    let mut conn = pool.acquire().await?;
    require_project_admin(&mut conn, project_id, actor).await?;
    let rows = sqlx::query_as::<_, (Uuid, String)>(
        "SELECT u.id, u.display_name
         FROM penfii.project_teams pt
         JOIN penfii.teams t ON t.id = pt.team_id
         JOIN penfii.team_members tm ON tm.team_id = pt.team_id
         JOIN penfii.projects p ON p.id = pt.project_id
         JOIN penfii.workspace_members wm
           ON wm.workspace_id = p.workspace_id AND wm.user_id = tm.user_id AND wm.state = 'ACTIVE'
         JOIN penfii.users u ON u.id = tm.user_id
         WHERE pt.project_id = $1 AND pt.team_id = $2 AND t.archived_at IS NULL
         ORDER BY u.display_name, u.id",
    )
    .bind(project_id)
    .bind(team_id)
    .fetch_all(&mut *conn)
    .await?;
    let linked: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM penfii.project_teams WHERE project_id = $1 AND team_id = $2)",
    )
    .bind(project_id)
    .bind(team_id)
    .fetch_one(&mut *conn)
    .await?;
    if !linked {
        return Err(CoreError::NotFound);
    }
    Ok(rows
        .into_iter()
        .map(|(user_id, display_name)| penfii_contracts::ProjectMember {
            user_id,
            display_name,
        })
        .collect())
}

/// A Project nobody can manage right now, for a Workspace owner or admin who may name an admin for it.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectWithoutAdmin {
    pub id: Uuid,
    pub name: String,
    pub ticket_prefix: String,
    /// Who held the admin role directly and cannot act now (left the Workspace, or account disabled).
    pub former_admins: Vec<FormerAdmin>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormerAdmin {
    pub user_id: Uuid,
    pub display_name: String,
}

/// The Projects of the Workspace that have no active admin, by any source, oldest first. Workspace
/// owners and admins only; archived Projects are left out. Only names are shown, never content.
pub async fn projects_without_admin(
    pool: &PgPool,
    actor: Uuid,
    workspace_id: Uuid,
) -> Result<Vec<ProjectWithoutAdmin>, CoreError> {
    let mut conn = pool.acquire().await?;
    crate::admin::require_manager(&mut conn, workspace_id, actor).await?;
    let rows = sqlx::query_as::<_, (Uuid, String, String, sqlx::types::Json<Vec<FormerAdminRow>>)>(&format!(
        "SELECT p.id, p.name, p.ticket_prefix,
                COALESCE((SELECT jsonb_agg(jsonb_build_object('userId', u.id, 'displayName', u.display_name)
                                           ORDER BY u.display_name, u.id)
                          FROM penfii.project_user_grants g JOIN penfii.users u ON u.id = g.user_id
                          WHERE g.project_id = p.id AND g.role IN ('ADMIN', 'OWNER')), '[]'::jsonb)
         FROM penfii.projects p
         WHERE p.workspace_id = $1 AND p.archived_at IS NULL
           AND NOT EXISTS (SELECT 1 FROM {ACTIVE_ADMIN_SOURCES} a WHERE a.project_id = p.id)
         ORDER BY p.created_at, p.id"
    ))
    .bind(workspace_id)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, name, ticket_prefix, former)| ProjectWithoutAdmin {
            id,
            name,
            ticket_prefix,
            former_admins: former
                .0
                .into_iter()
                .map(|f| FormerAdmin {
                    user_id: f.user_id,
                    display_name: f.display_name,
                })
                .collect(),
        })
        .collect())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct FormerAdminRow {
    user_id: Uuid,
    display_name: String,
}

/// What a recovery did.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminRecovery {
    /// The named person had no role in the Project before, so this gave them access to its content.
    pub new_access: bool,
}

/// A Workspace owner or admin names a direct admin for a Project that has no active admin. It is the
/// only thing this allows: it gives the person who names no powers in the Project (naming themself
/// is the one way they get any), nobody is promoted on their own, and a Project that has an active
/// admin by any source is refused (`project_has_admin`). The named person must be an active member
/// with an account that is not disabled. The Project is locked, so two recoveries at once name one.
/// Audited as `project.admin_recovered`, saying whether this opened the Project's content to someone
/// who could not read it.
pub async fn recover_admin(
    pool: &PgPool,
    actor: Uuid,
    project_id: Uuid,
    user_id: Uuid,
) -> Result<AdminRecovery, CoreError> {
    let mut tx = pool.begin().await?;
    let workspace: Uuid =
        sqlx::query_scalar("SELECT workspace_id FROM penfii.projects WHERE id = $1 FOR UPDATE")
            .bind(project_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(CoreError::NotFound)?;
    crate::admin::require_manager(&mut tx, workspace, actor).await?;
    if has_active_admin(&mut tx, project_id, None).await? {
        return Err(CoreError::conflict(
            "project_has_admin",
            "the Project has an active admin; only a Project without one is recovered this way",
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
    let (_, before) = project_role(&mut tx, project_id, user_id).await?;
    sqlx::query(
        "INSERT INTO penfii.project_user_grants (project_id, user_id, role) VALUES ($1, $2, 'ADMIN')
         ON CONFLICT (project_id, user_id) DO UPDATE SET role = 'ADMIN',
            can_waive_dependencies = false, can_manage_pins = false",
    )
    .bind(project_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    let new_access = before.is_none();
    audit(
        &mut tx,
        workspace,
        Some(actor),
        "project.admin_recovered",
        "PROJECT",
        project_id,
        serde_json::json!({
            "userId": user_id,
            "self": actor == user_id,
            "newAccess": new_access,
            "previousRole": before,
        }),
    )
    .await?;
    tx.commit().await?;
    Ok(AdminRecovery { new_access })
}

/// A Project that would be left without its direct admin, or without any admin, if the person left.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectImpact {
    pub project_id: Uuid,
    pub name: String,
    /// `NO_ADMIN`: nobody who can act would be left. `NO_DIRECT_ADMIN`: the last direct admin goes,
    /// but an admin through a Team or Group stays.
    pub kind: String,
}

/// The Projects the person is the last active admin of, in the sense of [`ProjectImpact`]. For the
/// warning before removing a member or disabling an account, and for the operator's `user-disable`.
/// Archived Projects are left out. It only informs: nothing is refused because of it.
pub(crate) async fn offboarding_impact_in(
    conn: &mut PgConnection,
    workspace_id: Uuid,
    user_id: Uuid,
) -> Result<Vec<ProjectImpact>, CoreError> {
    let rows = sqlx::query_as::<_, (Uuid, String, String)>(&format!(
        "SELECT p.id, p.name,
                CASE WHEN NOT EXISTS (SELECT 1 FROM {a} x WHERE x.project_id = p.id AND x.user_id <> $2)
                     THEN 'NO_ADMIN' ELSE 'NO_DIRECT_ADMIN' END
         FROM penfii.projects p
         WHERE p.workspace_id = $1 AND p.archived_at IS NULL
           AND (
                (EXISTS (SELECT 1 FROM {a} x WHERE x.project_id = p.id AND x.source = 'DIRECT' AND x.user_id = $2)
                 AND NOT EXISTS (SELECT 1 FROM {a} x WHERE x.project_id = p.id AND x.source = 'DIRECT' AND x.user_id <> $2))
                OR
                (EXISTS (SELECT 1 FROM {a} x WHERE x.project_id = p.id AND x.user_id = $2)
                 AND NOT EXISTS (SELECT 1 FROM {a} x WHERE x.project_id = p.id AND x.user_id <> $2))
           )
         ORDER BY p.name, p.id",
        a = ACTIVE_ADMIN_SOURCES
    ))
    .bind(workspace_id)
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(project_id, name, kind)| ProjectImpact {
            project_id,
            name,
            kind,
        })
        .collect())
}

/// `GET .../members/{user_id}/offboarding-impact`: what removing the member or disabling their
/// account would leave without an admin. Workspace owners and admins only.
pub async fn offboarding_impact(
    pool: &PgPool,
    actor: Uuid,
    workspace_id: Uuid,
    user_id: Uuid,
) -> Result<Vec<ProjectImpact>, CoreError> {
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
