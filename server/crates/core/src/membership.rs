//! Joining a Workspace: the one rule behind every way a person becomes a member.
//!
//! An accepted invitation and the operator's `admin-cli` both end here, so they cannot drift apart.
//! The rule, for an existing account and a Workspace:
//! - no membership yet: a new `ACTIVE` membership with the given role ([`JoinOutcome::Joined`]);
//! - an `INACTIVE` (removed) membership: it becomes `ACTIVE` again with the given role, and the
//!   earlier role is not brought back ([`JoinOutcome::Rejoined`]); channel memberships left over
//!   from before the removal are cleared;
//! - an `ACTIVE` membership: nothing changes, not even the role, and nothing is audited
//!   ([`JoinOutcome::AlreadyMember`]). Changing the role of a member is a separate action;
//! - a disabled account never joins; it has to be enabled first (`account_disabled`).
//!
//! The roles are `OWNER`, `ADMIN` and `MEMBER`. Who may hand out which role is for the caller to
//! decide (an invitation: only an owner invites an owner or admin; the operator: anyone), and so is
//! what a duplicate means to the caller: an invitation that finds the person already in is still
//! accepted, the operator is told.
//!
//! A rejoin brings back the Workspace role and nothing that was explicitly given before: removal
//! revokes the person's Team and Group memberships, direct Project grants and invited Doc participant
//! rows in that Workspace ([`revoke_explicit_access`]), as it clears channel memberships, so the person
//! comes back as a new member and is given access again. What they wrote, their direct messages and
//! the audit trail stay, as does access that comes from having written a Doc.
//!
//! A join is audited as `member.joined` in the caller's transaction. [`JoinPath`] says how the person
//! came in: through an invitation the person is their own actor, through the operator there is no
//! actor and the entry says `via: admin-cli`.

use serde_json::json;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::admin;
use crate::error::CoreError;

/// How a person comes into the Workspace; it only decides what the audit entry records.
#[derive(Clone, Copy, Debug)]
pub enum JoinPath {
    /// Accepting the invitation with this id; the joining person is the actor.
    Invitation(Uuid),
    /// The operator's command line: no actor, `via: admin-cli`.
    AdminCli,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JoinOutcome {
    /// A membership was created.
    Joined,
    /// A removed (`INACTIVE`) membership was made `ACTIVE` again with the given role.
    Rejoined,
    /// The person is already an `ACTIVE` member with this role; nothing was changed.
    AlreadyMember { role: String },
}

impl JoinOutcome {
    /// Whether the person is a new active member of the Workspace.
    pub fn changed(&self) -> bool {
        !matches!(self, Self::AlreadyMember { .. })
    }
}

/// The role if it is one a member can hold.
pub fn validate_role(role: &str) -> Result<&str, CoreError> {
    if matches!(role, "OWNER" | "ADMIN" | "MEMBER") {
        Ok(role)
    } else {
        Err(CoreError::validation(
            "role must be OWNER, ADMIN, or MEMBER",
        ))
    }
}

/// What [`revoke_explicit_access`] took away.
#[derive(Debug, Default, Clone)]
pub(crate) struct RevokedAccess {
    pub teams: i64,
    pub projects: i64,
    pub groups: i64,
    pub doc_shares: i64,
    /// Rows of `workspace_security_auditors` (read access to the audit log) for this Workspace.
    pub security_auditor: i64,
    /// Rows of `channel_content_auditors` (read access to private channel content) in this Workspace.
    pub channel_auditors: i64,
    /// The Projects where the person was a direct admin; the audit entry keeps them, so that the list of
    /// Projects without an admin can still name who held the role.
    pub direct_admin_of: Vec<Uuid>,
}

impl RevokedAccess {
    fn to_json(&self) -> serde_json::Value {
        json!({
            "teams": self.teams,
            "projects": self.projects,
            "groups": self.groups,
            "docShares": self.doc_shares,
            "securityAuditor": self.security_auditor,
            "channelAuditors": self.channel_auditors,
        })
    }

    /// Whether anything was taken away.
    fn any(&self) -> bool {
        self.teams > 0
            || self.projects > 0
            || self.groups > 0
            || self.doc_shares > 0
            || self.security_auditor > 0
            || self.channel_auditors > 0
    }

    pub(crate) fn audit_with(&self, mut data: serde_json::Value) -> serde_json::Value {
        data["revoked"] = self.to_json();
        data["directAdminOf"] = json!(self.direct_admin_of);
        data
    }
}

/// Takes away what was explicitly given to the person in this Workspace: Team and Group memberships,
/// direct Project grants and invited Doc participant rows (FEA-149). Nothing the person made or
/// belongs to by what they did is touched: not their messages, Docs, Tickets or audit entries, not the
/// participant row on a Doc they created, not their direct messages, and nothing in another Workspace.
/// Channel memberships are cleared by `conversation::recovery::clear_memberships`. The person's rows
/// as a Workspace security auditor and as a private channel's content auditor go too. A Doc page left
/// without a manager keeps its content; a Workspace owner or admin names a manager for it
/// ([`crate::docs::recovery`]). The same rules, for every removed person at once, are in migration
/// 0003.
/// Runs in the caller's transaction, after the membership has been made `INACTIVE`.
pub(crate) async fn revoke_explicit_access(
    conn: &mut PgConnection,
    workspace_id: Uuid,
    user_id: Uuid,
) -> Result<RevokedAccess, CoreError> {
    let teams = sqlx::query(
        "DELETE FROM penfii.team_members tm USING penfii.teams t
         WHERE tm.team_id = t.id AND t.workspace_id = $1 AND tm.user_id = $2",
    )
    .bind(workspace_id)
    .bind(user_id)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    let revoked_grants: Vec<(Uuid, String)> = sqlx::query_as(
        "DELETE FROM penfii.project_user_grants g USING penfii.projects p
         WHERE g.project_id = p.id AND p.workspace_id = $1 AND g.user_id = $2
         RETURNING g.project_id, g.role",
    )
    .bind(workspace_id)
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await?;
    let projects = revoked_grants.len() as u64;
    let direct_admin_of = revoked_grants
        .into_iter()
        .filter(|(_, role)| role == "ADMIN" || role == "OWNER")
        .map(|(project, _)| project)
        .collect();
    let groups = sqlx::query(
        "DELETE FROM penfii.group_members gm USING penfii.groups gr
         WHERE gm.group_id = gr.id AND gr.workspace_id = $1 AND gm.user_id = $2",
    )
    .bind(workspace_id)
    .bind(user_id)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    // Every participant row the person was given goes, a MANAGER row too, even when no other manager
    // would remain: the page stays and a Workspace owner or admin names a manager for it
    // (`docs::recovery`). Only the row on a page the person created stays (author-based access).
    let doc_shares = sqlx::query(
        "DELETE FROM penfii.doc_page_participants pp USING penfii.doc_pages p
         WHERE pp.page_id = p.id AND p.workspace_id = $1 AND pp.user_id = $2
           AND p.created_by <> pp.user_id",
    )
    .bind(workspace_id)
    .bind(user_id)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    let security_auditor = sqlx::query(
        "DELETE FROM penfii.workspace_security_auditors WHERE workspace_id = $1 AND user_id = $2",
    )
    .bind(workspace_id)
    .bind(user_id)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    let channel_auditors = sqlx::query(
        "DELETE FROM penfii.channel_content_auditors a USING penfii.conversations c
         WHERE a.channel_id = c.id AND c.workspace_id = $1 AND a.user_id = $2",
    )
    .bind(workspace_id)
    .bind(user_id)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    let count = |n: u64| i64::try_from(n).unwrap_or(i64::MAX);
    Ok(RevokedAccess {
        teams: count(teams),
        projects: count(projects),
        groups: count(groups),
        doc_shares: count(doc_shares),
        security_auditor: count(security_auditor),
        channel_auditors: count(channel_auditors),
        direct_admin_of,
    })
}

/// Makes the account a member of the Workspace under the rule above, inside the caller's
/// transaction. Both must exist.
pub async fn join_workspace(
    conn: &mut PgConnection,
    workspace_id: Uuid,
    user_id: Uuid,
    role: &str,
    path: JoinPath,
) -> Result<JoinOutcome, CoreError> {
    let role = validate_role(role)?;
    let workspace: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM penfii.workspaces WHERE id = $1")
            .bind(workspace_id)
            .fetch_optional(&mut *conn)
            .await?;
    if workspace.is_none() {
        return Err(CoreError::NotFound);
    }
    // Share-locked, so disabling the account (which locks the row) is ordered before or after the join.
    let disabled: Option<bool> = sqlx::query_scalar(
        "SELECT disabled_at IS NOT NULL FROM penfii.users WHERE id = $1 FOR SHARE",
    )
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?;
    match disabled {
        None => return Err(CoreError::NotFound),
        Some(true) => {
            return Err(CoreError::conflict(
                "account_disabled",
                "the account is disabled; enable it before adding it to a workspace",
            ));
        }
        Some(false) => {}
    }
    // One statement decides: it inserts, or revives a removed membership, and leaves an active one
    // alone, so two joins at once cannot both claim the change.
    let changed: Option<bool> = sqlx::query_scalar(
        "INSERT INTO penfii.workspace_members (workspace_id, user_id, role, state)
         VALUES ($1, $2, $3, 'ACTIVE')
         ON CONFLICT (workspace_id, user_id) DO UPDATE
            SET state = 'ACTIVE', role = EXCLUDED.role, deactivated_at = NULL, joined_at = now()
            WHERE penfii.workspace_members.state = 'INACTIVE'
         RETURNING (xmax = 0)",
    )
    .bind(workspace_id)
    .bind(user_id)
    .bind(role)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(inserted) = changed else {
        let current: String = sqlx::query_scalar(
            "SELECT role FROM penfii.workspace_members WHERE workspace_id = $1 AND user_id = $2",
        )
        .bind(workspace_id)
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
        return Ok(JoinOutcome::AlreadyMember { role: current });
    };
    let mut revoked = None;
    if !inserted {
        // A person who was removed before channel memberships were cleared on removal can still
        // have them; they are not brought back with the rest of the membership.
        crate::conversation::recovery::clear_memberships(&mut *conn, workspace_id, user_id).await?;
        // Likewise for explicit access that removal in an earlier release left behind.
        revoked = Some(revoke_explicit_access(&mut *conn, workspace_id, user_id).await?);
    }
    let (actor, mut data) = match path {
        JoinPath::Invitation(invitation_id) => (
            Some(user_id),
            json!({"userId": user_id, "role": role, "invitationId": invitation_id, "rejoined": !inserted}),
        ),
        JoinPath::AdminCli => (
            None,
            json!({"userId": user_id, "role": role, "via": "admin-cli", "rejoined": !inserted}),
        ),
    };
    // Access left over from an earlier release and taken away now is recorded with the join.
    if let Some(revoked) = revoked.filter(RevokedAccess::any) {
        data["revoked"] = revoked.to_json();
    }
    admin::audit(
        &mut *conn,
        workspace_id,
        actor,
        "member.joined",
        "USER",
        user_id,
        data,
    )
    .await?;
    Ok(if inserted {
        JoinOutcome::Joined
    } else {
        JoinOutcome::Rejoined
    })
}

/// The operator's way in: joins the account to the Workspace in a transaction of its own, audited
/// as done through `admin-cli`. The same rule as an accepted invitation, without an invitation.
pub async fn join_workspace_as_operator(
    pool: &PgPool,
    workspace_id: Uuid,
    user_id: Uuid,
    role: &str,
) -> Result<JoinOutcome, CoreError> {
    let mut tx = pool.begin().await?;
    let outcome = join_workspace(&mut tx, workspace_id, user_id, role, JoinPath::AdminCli).await?;
    tx.commit().await?;
    Ok(outcome)
}

/// What the operator's `create-user` and `workspace-add-member` did.
#[derive(Debug)]
pub struct OperatorJoin {
    pub user_id: Uuid,
    pub workspace_id: Uuid,
    pub outcome: JoinOutcome,
}

/// `admin-cli create-user`: makes a new account (verified, with a password) and joins it to an
/// existing Workspace in one transaction, by the same rule as every other join. Nothing is made when
/// the Workspace does not exist, the e-mail already has an account (`account_exists`; that account
/// is added with [`operator_add_member`]) or the password or name is refused.
pub async fn operator_create_member(
    pool: &PgPool,
    workspace_id: Uuid,
    email: &str,
    password: &str,
    display_name: &str,
    role: &str,
) -> Result<OperatorJoin, CoreError> {
    let role = validate_role(role)?;
    let (email, email_key) = crate::identity::normalize_email(email)?;
    crate::identity::validate_new_password(password, &email)?;
    let display_name = crate::identity::require_label(display_name, "displayName")?;
    // Hashed before the transaction opens: no connection is held while Argon2 runs.
    let password_hash = crate::identity::hash_password(password).await?;
    let mut tx = pool.begin().await?;
    let workspace: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM penfii.workspaces WHERE id = $1")
            .bind(workspace_id)
            .fetch_optional(&mut *tx)
            .await?;
    if workspace.is_none() {
        return Err(CoreError::NotFound);
    }
    let user_id = crate::identity::insert_user(
        &mut tx,
        &email,
        &email_key,
        &display_name,
        &password_hash,
        true,
    )
    .await?;
    let outcome = join_workspace(&mut tx, workspace_id, user_id, role, JoinPath::AdminCli).await?;
    tx.commit().await?;
    Ok(OperatorJoin {
        user_id,
        workspace_id,
        outcome,
    })
}

/// `admin-cli workspace-add-member`: joins the account that has this e-mail to an existing
/// Workspace, by the rule above (`AlreadyMember` is for the caller to report). An e-mail without an
/// account is `account_not_found`; such a person is made with [`operator_create_member`].
pub async fn operator_add_member(
    pool: &PgPool,
    workspace_id: Uuid,
    email: &str,
    role: &str,
) -> Result<OperatorJoin, CoreError> {
    let role = validate_role(role)?;
    let (_, email_key) = crate::identity::normalize_email(email)?;
    let mut tx = pool.begin().await?;
    let user_id: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM penfii.users WHERE email_key = $1")
            .bind(&email_key)
            .fetch_optional(&mut *tx)
            .await?;
    let Some(user_id) = user_id else {
        return Err(CoreError::rule(
            "account_not_found",
            "no account has this e-mail; create it with create-user",
        ));
    };
    let outcome = join_workspace(&mut tx, workspace_id, user_id, role, JoinPath::AdminCli).await?;
    tx.commit().await?;
    Ok(OperatorJoin {
        user_id,
        workspace_id,
        outcome,
    })
}
