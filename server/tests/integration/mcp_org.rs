//! MCP organisation operations (FEA-169) and the web's recent-sign-in rule for OWNER/ADMIN role
//! changes: teams, departments, members and MEMBER invitations through MCP; no link ever returned;
//! the web-only actions are not tools.

use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use crate::common::{OWNER_EMAIL, OWNER_PASSWORD, sign_in};
use crate::mcp_oauth::{
    Env, add_member, error_code, modern_call, modern_list, rest, setup, token_for, tool_names,
};
use crate::mcp_support::{mcp_err, mcp_ok, user_id};

const ORG_SCOPES: &str = "workspace:read workspace:write member:read member:write team:write";

async fn age_sessions(env: &Env) {
    sqlx::query("UPDATE penfii.sessions SET last_reauth_at = now() - interval '2 hours', authenticated_at = now() - interval '2 hours'")
        .execute(&env.pool)
        .await
        .unwrap();
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn owner_and_admin_role_changes_in_the_web_need_a_recent_sign_in(pool: PgPool) {
    let env = setup(pool).await;
    let _member_cookie = add_member(&env, "mina@penfii.test", "MEMBER").await;
    let mina = user_id(&env, "mina@penfii.test").await;
    let path = format!("/v1/workspaces/{}/members/{mina}", env.workspace_id);
    let role_of = |env: &Env| {
        let pool = env.pool.clone();
        let workspace = env.workspace_id;
        async move {
            sqlx::query_scalar::<_, String>(
                "SELECT role FROM penfii.workspace_members WHERE workspace_id = $1 AND user_id = $2",
            )
            .bind(workspace)
            .bind(mina)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };

    // A session that signed in long ago is refused, and nothing changes.
    age_sessions(&env).await;
    let stale = rest(
        &env,
        "PATCH",
        &path,
        &env.owner_cookie,
        None,
        Some(json!({"role": "ADMIN"})),
    )
    .await;
    assert_eq!(stale.status, 403, "{}", stale.body);
    assert_eq!(stale.body["code"], "reauth_required");
    assert_eq!(role_of(&env).await, "MEMBER");

    // After signing in again the same request works; taking the role back needs it too.
    let fresh = sign_in(&env.app, OWNER_EMAIL, OWNER_PASSWORD).await;
    let promoted = rest(
        &env,
        "PATCH",
        &path,
        &fresh,
        None,
        Some(json!({"role": "ADMIN"})),
    )
    .await;
    assert_eq!(promoted.status, 204, "{}", promoted.body);
    assert_eq!(role_of(&env).await, "ADMIN");
    age_sessions(&env).await;
    let stale = rest(
        &env,
        "PATCH",
        &path,
        &fresh,
        None,
        Some(json!({"role": "MEMBER"})),
    )
    .await;
    assert_eq!(stale.body["code"], "reauth_required", "{}", stale.body);
    assert_eq!(role_of(&env).await, "ADMIN");
    // The same role again changes nothing and asks for nothing.
    let same = rest(
        &env,
        "PATCH",
        &path,
        &fresh,
        None,
        Some(json!({"role": "ADMIN"})),
    )
    .await;
    assert_eq!(same.status, 204, "{}", same.body);

    // Someone who may not change roles is told so, not asked to sign in again.
    let admin_cookie = sign_in(&env.app, "mina@penfii.test", "member-test-passphrase").await;
    age_sessions(&env).await;
    let forbidden = rest(
        &env,
        "PATCH",
        &path,
        &admin_cookie,
        None,
        Some(json!({"role": "MEMBER"})),
    )
    .await;
    assert_eq!(forbidden.status, 403);
    assert_ne!(
        forbidden.body["code"], "reauth_required",
        "{}",
        forbidden.body
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn teams_departments_and_members_are_managed_like_in_the_web(pool: PgPool) {
    let env = setup(pool).await;
    add_member(&env, "mina@penfii.test", "MEMBER").await;
    let mina = user_id(&env, "mina@penfii.test").await;
    let token = token_for(&env, &env.owner_cookie, ORG_SCOPES).await;
    let ws = env.workspace_id;

    let department = mcp_ok(
        &env,
        &token,
        "department_create",
        json!({"workspaceId": ws, "name": "Eng"}),
    )
    .await;
    let department_id = department["id"].as_str().unwrap().to_string();
    let team = mcp_ok(
        &env,
        &token,
        "team_create",
        json!({"workspaceId": ws, "name": "Core",
        "description": "core team", "members": [{"userId": env.owner_id, "role": "MANAGER"}]}),
    )
    .await;
    let team_id = team["id"].as_str().unwrap().to_string();
    mcp_ok(
        &env,
        &token,
        "team_update",
        json!({"workspaceId": ws, "teamId": team_id, "description": null}),
    )
    .await;
    let added = mcp_ok(
        &env,
        &token,
        "team_member_set",
        json!({"workspaceId": ws, "teamId": team_id, "userId": mina, "role": "MEMBER"}),
    )
    .await;
    assert_eq!(added["added"], true);
    let members = mcp_ok(
        &env,
        &token,
        "team_members",
        json!({"workspaceId": ws, "teamId": team_id}),
    )
    .await;
    assert_eq!(members["items"].as_array().unwrap().len(), 2);
    let web = rest(
        &env,
        "GET",
        &format!("/v1/workspaces/{ws}/teams/{team_id}/members"),
        &env.owner_cookie,
        None,
        None,
    )
    .await;
    assert_eq!(members["items"], web.body["items"]);

    // Core's rules come through: the last manager cannot be removed or demoted.
    assert!(
        !mcp_err(
            &env,
            &token,
            "team_member_remove",
            json!({"workspaceId": ws, "teamId": team_id, "userId": env.owner_id})
        )
        .await
        .is_empty()
    );
    mcp_ok(
        &env,
        &token,
        "team_member_remove",
        json!({"workspaceId": ws, "teamId": team_id, "userId": mina}),
    )
    .await;

    mcp_ok(
        &env,
        &token,
        "department_place_team",
        json!({"workspaceId": ws, "teamId": team_id, "departmentId": department_id}),
    )
    .await;
    let departments = mcp_ok(&env, &token, "department_list", json!({"workspaceId": ws})).await;
    let web = rest(
        &env,
        "GET",
        &format!("/v1/workspaces/{ws}/departments"),
        &env.owner_cookie,
        None,
        None,
    )
    .await;
    assert_eq!(departments["items"], web.body["items"]);
    mcp_ok(
        &env,
        &token,
        "team_archive",
        json!({"workspaceId": ws, "teamId": team_id}),
    )
    .await;
    mcp_ok(
        &env,
        &token,
        "team_restore",
        json!({"workspaceId": ws, "teamId": team_id}),
    )
    .await;
    mcp_ok(
        &env,
        &token,
        "department_delete",
        json!({"workspaceId": ws, "departmentId": department_id}),
    )
    .await;
    let renamed = mcp_ok(
        &env,
        &token,
        "workspace_update",
        json!({"workspaceId": ws, "name": "Renamed WS"}),
    )
    .await;
    assert_eq!(renamed["name"], "Renamed WS");

    let roster = mcp_ok(
        &env,
        &token,
        "member_list",
        json!({"workspaceId": ws, "limit": 1}),
    )
    .await;
    assert_eq!(roster["items"].as_array().unwrap().len(), 1);
    assert!(
        roster["nextCursor"].is_string() && roster["total"] == 2,
        "{roster}"
    );
    let directory = mcp_ok(&env, &token, "member_directory", json!({"workspaceId": ws})).await;
    assert_eq!(directory["items"].as_array().unwrap().len(), 2);
    mcp_ok(
        &env,
        &token,
        "member_offboarding_impact",
        json!({"workspaceId": ws, "userId": mina}),
    )
    .await;

    // The audit log has the same entries the web route made, with the real actor.
    let audit = mcp_ok(
        &env,
        &token_for(&env, &env.owner_cookie, "admin:read").await,
        "audit_list",
        json!({"workspaceId": ws, "action": "team."}),
    )
    .await;
    assert!(
        audit["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["action"] == "team.member_added"),
        "{audit}"
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn roles_scopes_and_policy_limit_organisation_tools(pool: PgPool) {
    let env = setup(pool).await;
    let ws = env.workspace_id;
    let member_cookie = add_member(&env, "mina@penfii.test", "MEMBER").await;
    let member = token_for(&env, &member_cookie, ORG_SCOPES).await;
    let outsider = token_for(&env, &env.outsider_cookie, ORG_SCOPES).await;
    // (A new consent replaces the person's grant for the client: one owner token at a time.)
    let reader = token_for(&env, &env.owner_cookie, "workspace:read member:read").await;
    let denied = modern_call(
        &env.app,
        &reader,
        "department_create",
        json!({"workspaceId": ws, "name": "X"}),
    )
    .await;
    assert_eq!(error_code(&denied), "insufficient_scope");
    let owner = token_for(&env, &env.owner_cookie, ORG_SCOPES).await;

    // A plain member may read the directory, not manage.
    mcp_ok(
        &env,
        &member,
        "member_directory",
        json!({"workspaceId": ws}),
    )
    .await;
    assert_eq!(
        mcp_err(
            &env,
            &member,
            "team_create",
            json!({"workspaceId": ws, "name": "X",
        "members": [{"userId": env.owner_id, "role": "MANAGER"}]})
        )
        .await,
        "forbidden"
    );
    assert_eq!(
        mcp_err(
            &env,
            &member,
            "invitation_create",
            json!({"workspaceId": ws, "email": "a@penfii.test"})
        )
        .await,
        "forbidden"
    );
    assert_eq!(
        mcp_err(&env, &member, "member_list", json!({"workspaceId": ws})).await,
        "forbidden"
    );
    // Another Workspace's person sees nothing.
    assert_eq!(
        mcp_err(
            &env,
            &outsider,
            "member_directory",
            json!({"workspaceId": ws})
        )
        .await,
        "not_found"
    );
    assert_eq!(
        mcp_err(
            &env,
            &outsider,
            "department_create",
            json!({"workspaceId": ws, "name": "X"})
        )
        .await,
        "not_found"
    );
    // Policy: writes off.
    sqlx::query("INSERT INTO penfii.workspace_mcp_policy (workspace_id, mcp_enabled, allow_write, allow_destructive, allow_administrative, updated_by)
                 VALUES ($1, true, false, false, false, $2)")
        .bind(ws).bind(env.owner_id).execute(&env.pool).await.unwrap();
    assert_eq!(
        mcp_err(
            &env,
            &owner,
            "department_create",
            json!({"workspaceId": ws, "name": "X"})
        )
        .await,
        "policy_denied"
    );
    mcp_ok(&env, &owner, "department_list", json!({"workspaceId": ws})).await;
    // workspace_create needs a connection that is not limited to chosen Workspaces and Core's consent.
    sqlx::query("DELETE FROM penfii.workspace_mcp_policy")
        .execute(&env.pool)
        .await
        .unwrap();
    assert_eq!(
        mcp_err(
            &env,
            &owner,
            "workspace_create",
            json!({"name": "Second", "timeZone": "Asia/Seoul"})
        )
        .await,
        "workspace_creation_closed"
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn invitations_through_mcp_are_member_only_and_never_carry_the_link(pool: PgPool) {
    let env = setup(pool).await;
    let ws = env.workspace_id;
    let token = token_for(&env, &env.owner_cookie, ORG_SCOPES).await;

    let created = mcp_ok(
        &env,
        &token,
        "invitation_create",
        json!({"workspaceId": ws, "email": "new@penfii.test"}),
    )
    .await;
    assert_eq!(created["role"], "MEMBER");
    assert_eq!(created["emailStatus"], "NOT_CONFIGURED", "{created}");
    let invitation_id = created["id"].as_str().unwrap().to_string();
    // A retry re-issues the same invitation instead of making a second one.
    let again = mcp_ok(
        &env,
        &token,
        "invitation_create",
        json!({"workspaceId": ws, "email": "new@penfii.test"}),
    )
    .await;
    assert_eq!(again["id"], created["id"]);
    assert_eq!(again["reissued"], true);

    // OWNER/ADMIN invitations are not something MCP does, whichever way it is asked.
    assert_eq!(
        mcp_err(
            &env,
            &token,
            "invitation_create",
            json!({"workspaceId": ws, "email": "boss@penfii.test", "role": "ADMIN"})
        )
        .await,
        "web_only"
    );
    let admin_invite = rest(
        &env,
        "POST",
        &format!("/v1/workspaces/{ws}/invitations"),
        &env.owner_cookie,
        Some("inv-admin"),
        Some(json!({"email": "admin-to-be@penfii.test", "role": "ADMIN"})),
    )
    .await;
    assert_eq!(admin_invite.status, 201, "{}", admin_invite.body);
    let secret = admin_invite.body["inviteToken"]
        .as_str()
        .unwrap()
        .to_string();
    let admin_invitation_id = admin_invite.body["id"].as_str().unwrap().to_string();
    assert_eq!(
        mcp_err(
            &env,
            &token,
            "invitation_reissue",
            json!({"workspaceId": ws, "invitationId": admin_invitation_id})
        )
        .await,
        "web_only"
    );

    let listed = mcp_ok(&env, &token, "invitation_list", json!({"workspaceId": ws})).await;
    assert_eq!(listed["total"], 2);
    let reissued = mcp_ok(
        &env,
        &token,
        "invitation_reissue",
        json!({"workspaceId": ws, "invitationId": invitation_id}),
    )
    .await;
    assert_eq!(reissued["id"], created["id"]);

    // No secret anywhere: replies, the call log, the audit log.
    let mut haystack = vec![
        created.to_string(),
        again.to_string(),
        listed.to_string(),
        reissued.to_string(),
    ];
    let log: Vec<String> =
        sqlx::query_scalar("SELECT row_to_json(l)::text FROM penfii.mcp_call_log l")
            .fetch_all(&env.pool)
            .await
            .unwrap();
    haystack.extend(log);
    let audit: Vec<String> =
        sqlx::query_scalar("SELECT row_to_json(a)::text FROM penfii.audit_entries a")
            .fetch_all(&env.pool)
            .await
            .unwrap();
    haystack.extend(audit);
    for text in &haystack {
        let lower = text.to_lowercase();
        assert!(
            !lower.contains("invitetoken") && !lower.contains("invite_token"),
            "{text}"
        );
        assert!(
            !text.contains("/invite/") && !text.contains(&secret),
            "{text}"
        );
    }

    // Revoking works through MCP and shows in the web list.
    mcp_ok(
        &env,
        &token,
        "invitation_revoke",
        json!({"workspaceId": ws, "invitationId": invitation_id}),
    )
    .await;
    let web = rest(
        &env,
        "GET",
        &format!("/v1/workspaces/{ws}/invitations"),
        &env.owner_cookie,
        None,
        None,
    )
    .await;
    let row = web.body["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["id"] == json!(invitation_id))
        .unwrap();
    assert_eq!(row["status"], "REVOKED");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_removed_member_invited_again_gets_none_of_the_old_access(pool: PgPool) {
    let env = setup(pool).await;
    let ws = env.workspace_id;
    let member_cookie = add_member(&env, "mina@penfii.test", "MEMBER").await;
    let mina = user_id(&env, "mina@penfii.test").await;
    let token = token_for(&env, &env.owner_cookie, ORG_SCOPES).await;
    let team = mcp_ok(&env, &token, "team_create", json!({"workspaceId": ws, "name": "T",
        "members": [{"userId": env.owner_id, "role": "MANAGER"}, {"userId": mina, "role": "MEMBER"}]})).await;
    let team_id: Uuid = team["id"].as_str().unwrap().parse().unwrap();

    // Forced removal is the web's (recent sign-in); MCP has no tool for it.
    let removed = rest(
        &env,
        "DELETE",
        &format!("/v1/workspaces/{ws}/members/{mina}"),
        &env.owner_cookie,
        None,
        None,
    )
    .await;
    assert_eq!(removed.status, 204, "{}", removed.body);
    let in_team = |env: &Env| {
        let pool = env.pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM penfii.team_members WHERE team_id = $1 AND user_id = $2",
            )
            .bind(team_id)
            .bind(mina)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    assert_eq!(
        in_team(&env).await,
        0,
        "removal takes the Team membership with it"
    );

    mcp_ok(
        &env,
        &token,
        "invitation_create",
        json!({"workspaceId": ws, "email": "mina@penfii.test"}),
    )
    .await;
    // The link is never returned, so the test plants a token whose hash it knows.
    sqlx::query("UPDATE penfii.workspace_invitations SET token_hash = $1 WHERE workspace_id = $2 AND email_key = 'mina@penfii.test'")
        .bind(Sha256::digest(b"known-test-token").to_vec()).bind(ws).execute(&env.pool).await.unwrap();
    let accepted = rest(
        &env,
        "POST",
        "/v1/invitations/accept",
        &member_cookie,
        Some("accept-1"),
        Some(json!({"token": "known-test-token", "displayName": "mina", "password": ""})),
    )
    .await;
    assert_eq!(accepted.status, 201, "{}", accepted.body);
    assert_eq!(in_team(&env).await, 0, "no old Team membership comes back");
    let role: String = sqlx::query_scalar(
        "SELECT role FROM penfii.workspace_members WHERE workspace_id = $1 AND user_id = $2",
    )
    .bind(ws)
    .bind(mina)
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(role, "MEMBER");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn web_only_actions_are_not_tools(pool: PgPool) {
    let env = setup(pool).await;
    let token = token_for(&env, &env.owner_cookie, ORG_SCOPES).await;
    let names = tool_names(&modern_list(&env.app, &token).await);
    for forbidden in [
        "member_set_role",
        "member_change_role",
        "member_remove",
        "member_disable",
        "workspace_set_mcp_policy",
    ] {
        assert!(
            !names.contains(&forbidden.to_string()),
            "{forbidden} must not be a tool"
        );
        let reply = modern_call(&env.app, &token, forbidden, json!({})).await;
        assert_eq!(reply.body["error"]["code"], -32602, "unknown tool");
    }
    // The tool that explains the hand-over exists and names where it happens.
    let web = mcp_ok(&env, &token, "workspace_web_only", json!({})).await;
    let text = web.to_string();
    assert!(
        text.contains("OWNER or ADMIN") && text.contains("webhook"),
        "{text}"
    );
}
