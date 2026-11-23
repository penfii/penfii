//! A Workspace or Team manager cannot use the Team roster to give themself a Project admin role they
//! did not have (FEA-148). Adding other people, roles below admin, and the Project's own recovery
//! of a Project without an admin stay as they were.

use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::common::{Env, Person, Reply};

async fn project(env: &Env, name: &str) -> String {
    let reply = env
        .call(
            &env.owner,
            "POST",
            &format!("/v1/workspaces/{}/projects", env.ws),
            Some(json!({"name": name, "teamIds": []})),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    reply.body["id"].as_str().unwrap().to_string()
}

/// A Team the owner manages (the owner is in it), nobody else.
async fn team(env: &Env) -> String {
    let reply = env
        .call(
            &env.owner,
            "POST",
            &format!("/v1/workspaces/{}/teams", env.ws),
            Some(
                json!({"name": "Leads", "members": [{"userId": env.owner.id, "role": "MANAGER"}]}),
            ),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    reply.body["id"].as_str().unwrap().to_string()
}

async fn link(env: &Env, project: &str, team: &str, role: Option<&str>) {
    let linked = env
        .call(
            &env.owner,
            "PUT",
            &format!("/v1/projects/{project}/teams/{team}"),
            None,
        )
        .await;
    assert!(linked.status.is_success(), "{}", linked.body);
    let reply = env
        .call(
            &env.owner,
            "PUT",
            &format!("/v1/projects/{project}/teams/{team}/participation"),
            Some(json!({"role": role})),
        )
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.body);
}

async fn add(env: &Env, by: &Person, team: &str, who: Uuid, role: &str) -> Reply {
    env.call(
        by,
        "PUT",
        &format!("/v1/workspaces/{}/teams/{team}/members/{who}", env.ws),
        Some(json!({"role": role})),
    )
    .await
}

async fn in_team(env: &Env, team: &str, who: Uuid) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM penfii.team_members WHERE team_id = $1 AND user_id = $2)",
    )
    .bind(Uuid::parse_str(team).unwrap())
    .bind(who)
    .fetch_one(&env.pool)
    .await
    .unwrap()
}

async fn reads(env: &Env, who: &Person, project: &str) -> bool {
    env.call(who, "GET", &format!("/v1/projects/{project}/tickets"), None)
        .await
        .status
        == StatusCode::OK
}

/// Project admins are the ones who read the permissions of a Project.
async fn administers(env: &Env, who: &Person, project: &str) -> bool {
    env.call(
        who,
        "GET",
        &format!("/v1/projects/{project}/permissions"),
        None,
    )
    .await
    .status
        == StatusCode::OK
}

fn code(reply: &Reply) -> Option<&str> {
    reply.body["code"].as_str()
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn adding_yourself_to_a_team_that_makes_you_project_admin_is_refused(pool: PgPool) {
    let env = Env::new(pool).await;
    let admin = env.person("wsadmin", "ADMIN").await;
    let manager = env.person("manager", "MEMBER").await;
    let project = project(&env, "Alpha").await;
    let team = team(&env).await;
    link(&env, &project, &team, Some("ADMIN")).await;
    // The Team manager is a plain member; a Workspace admin is not in the Team either.
    assert_eq!(
        add(&env, &env.owner, &team, manager.id, "MANAGER")
            .await
            .status,
        StatusCode::NO_CONTENT
    );

    // The Workspace admin cannot add themself.
    let refused = add(&env, &admin, &team, admin.id, "MEMBER").await;
    assert_eq!(
        refused.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        refused.body
    );
    assert_eq!(code(&refused), Some("team_self_promotion"));
    assert!(!in_team(&env, &team, admin.id).await);
    assert!(!administers(&env, &admin, &project).await);

    // Another person adds them: ordinary, and the audit says what it gave.
    let by_other = add(&env, &manager, &team, admin.id, "MEMBER").await;
    assert_eq!(by_other.status, StatusCode::NO_CONTENT, "{}", by_other.body);
    assert!(in_team(&env, &team, admin.id).await);
    assert!(administers(&env, &admin, &project).await);
    let data: Value = sqlx::query_scalar(
        "SELECT data FROM penfii.audit_entries WHERE workspace_id = $1 AND action_code = 'team.member_added'
           AND data->>'userId' = $2",
    )
    .bind(env.ws)
    .bind(admin.id.to_string())
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(data["projectAdminGained"], json!([project]));
    // Someone already in the Team changes their own Team role without it counting as a promotion.
    assert_eq!(
        add(&env, &admin, &team, admin.id, "MANAGER").await.status,
        StatusCode::NO_CONTENT
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn self_joining_below_admin_or_with_admin_already_held_is_allowed(pool: PgPool) {
    let env = Env::new(pool).await;
    let admin = env.person("wsadmin", "ADMIN").await;
    let editor_project = project(&env, "Editors").await;
    let admin_project = project(&env, "Admins").await;
    let editor_team = team(&env).await;
    let admin_team = team(&env).await;
    link(&env, &editor_project, &editor_team, Some("EDITOR")).await;
    link(&env, &admin_project, &admin_team, Some("ADMIN")).await;

    // An editor/viewer/no-role Team gives no new admin role: the Workspace admin joins as before.
    let joined = add(&env, &admin, &editor_team, admin.id, "MEMBER").await;
    assert_eq!(joined.status, StatusCode::NO_CONTENT, "{}", joined.body);
    assert!(reads(&env, &admin, &editor_project).await);
    assert!(!administers(&env, &admin, &editor_project).await);

    // Already an admin of that Project by a direct grant: nothing new is gained.
    let grant = env
        .call(
            &env.owner,
            "PUT",
            &format!("/v1/projects/{admin_project}/grants/{}", admin.id),
            Some(json!({"role": "ADMIN"})),
        )
        .await;
    assert!(grant.status.is_success(), "{}", grant.body);
    let joined = add(&env, &admin, &admin_team, admin.id, "MEMBER").await;
    assert_eq!(joined.status, StatusCode::NO_CONTENT, "{}", joined.body);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn one_new_admin_project_among_several_refuses_and_the_group_behind_a_team_counts(
    pool: PgPool,
) {
    let env = Env::new(pool).await;
    let admin = env.person("wsadmin", "ADMIN").await;
    let held = project(&env, "Held").await;
    let new = project(&env, "New").await;
    let team = team(&env).await;
    link(&env, &held, &team, Some("ADMIN")).await;
    link(&env, &new, &team, Some("VIEWER")).await;
    env.call(
        &env.owner,
        "PUT",
        &format!("/v1/projects/{held}/grants/{}", admin.id),
        Some(json!({"role": "ADMIN"})),
    )
    .await;
    // Held is already administered by them, New gives the Team a viewer role only: allowed.
    assert_eq!(
        add(&env, &admin, &team, admin.id, "MEMBER").await.status,
        StatusCode::NO_CONTENT
    );
    sqlx::query("DELETE FROM penfii.team_members WHERE team_id = $1 AND user_id = $2")
        .bind(Uuid::parse_str(&team).unwrap())
        .bind(admin.id)
        .execute(&env.pool)
        .await
        .unwrap();
    // New is raised to admin: now one project is new admin for them, and the join is refused.
    link(&env, &new, &team, Some("ADMIN")).await;
    let refused = add(&env, &admin, &team, admin.id, "MEMBER").await;
    assert_eq!(
        code(&refused),
        Some("team_self_promotion"),
        "{}",
        refused.body
    );

    // The Group behind another Team carries the admin role of a third Project.
    let third = project(&env, "Third").await;
    let other_team = self::team(&env).await;
    let group = Uuid::new_v4();
    sqlx::query("INSERT INTO penfii.groups (id, workspace_id, name, system_team_id) VALUES ($1, $2, 'Ops', $3)")
        .bind(group)
        .bind(env.ws)
        .bind(Uuid::parse_str(&other_team).unwrap())
        .execute(&env.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO penfii.project_group_grants (project_id, group_id, role) VALUES ($1, $2, 'ADMIN')")
        .bind(Uuid::parse_str(&third).unwrap())
        .bind(group)
        .execute(&env.pool)
        .await
        .unwrap();
    let refused = add(&env, &admin, &other_team, admin.id, "MEMBER").await;
    assert_eq!(
        code(&refused),
        Some("team_self_promotion"),
        "{}",
        refused.body
    );
    // Putting someone else there is allowed.
    let other = env.person("other", "MEMBER").await;
    assert_eq!(
        add(&env, &env.owner, &other_team, other.id, "MEMBER")
            .await
            .status,
        StatusCode::NO_CONTENT
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn the_recovery_of_a_project_without_an_admin_still_names_the_caller(pool: PgPool) {
    let env = Env::new(pool).await;
    let admin = env.person("wsadmin", "ADMIN").await;
    let project = project(&env, "Orphan").await;
    sqlx::query("DELETE FROM penfii.project_user_grants WHERE project_id = $1")
        .bind(Uuid::parse_str(&project).unwrap())
        .execute(&env.pool)
        .await
        .unwrap();
    let recovered = env
        .call(
            &admin,
            "POST",
            &format!("/v1/projects/{project}/admin-recovery"),
            Some(json!({"userId": admin.id})),
        )
        .await;
    assert!(recovered.status.is_success(), "{}", recovered.body);
    assert!(administers(&env, &admin, &project).await);
}

/// An invitation's Team step does not place someone who is already an active member, so a Workspace
/// admin cannot invite an address, make it their own, and accept their way into a Team that makes them
/// a Project admin. An inviter coming back by their own invitation is held to the same rule.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn accepting_an_invitation_into_a_team_cannot_promote_yourself(pool: PgPool) {
    let env = Env::new(pool).await;
    let admin = env.person("wsadmin", "ADMIN").await;
    let project = project(&env, "Alpha").await;
    let team = team(&env).await;
    link(&env, &project, &team, Some("ADMIN")).await;

    let invite = |email: &'static str| {
        let (env, admin, team) = (&env, &admin, team.clone());
        async move {
            let reply = env
                .call(
                    admin,
                    "POST",
                    &format!("/v1/workspaces/{}/invitations", env.ws),
                    Some(json!({"email": email, "role": "MEMBER", "teamId": team})),
                )
                .await;
            assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
            reply.body["inviteToken"].clone()
        }
    };
    let accept = |who: &Person, token: Value| {
        let (env, cookie) = (&env, who.cookie.clone());
        async move {
            crate::common::call(
                &env.app,
                "POST",
                "/v1/invitations/accept",
                Some(&cookie),
                Some(json!({"token": token, "displayName": "me"})),
            )
            .await
        }
    };
    let set_email = |who: Uuid, email: &'static str| {
        let pool = env.pool.clone();
        async move {
            sqlx::query("UPDATE penfii.users SET email = $2, email_key = $2 WHERE id = $1")
                .bind(who)
                .bind(email)
                .execute(&pool)
                .await
                .unwrap();
        }
    };
    let skipped_reasons = || {
        let (pool, ws) = (env.pool.clone(), env.ws);
        async move {
            sqlx::query_scalar::<_, String>(
                "SELECT data->>'reason' FROM penfii.audit_entries
                  WHERE workspace_id = $1 AND action_code = 'invitation.team_skipped'
                  ORDER BY occurred_at",
            )
            .bind(ws)
            .fetch_all(&pool)
            .await
            .unwrap()
        }
    };

    // The admin invites an address, makes it theirs, and accepts while already an active member.
    let token = invite("alias@penfii.test").await;
    let own_token = invite("alias2@penfii.test").await;
    let fresh_token = invite("fresh@penfii.test").await;
    set_email(admin.id, "alias@penfii.test").await;
    let accepted = accept(&admin, token).await;
    assert_eq!(accepted.status, StatusCode::CREATED, "{}", accepted.body);
    assert_eq!(accepted.body["team"]["status"], "ALREADY_MEMBER");
    assert!(!in_team(&env, &team, admin.id).await);
    assert!(!administers(&env, &admin, &project).await);
    assert_eq!(skipped_reasons().await, vec!["ALREADY_MEMBER".to_string()]);

    // The same admin, removed, comes back by an invitation of their own: still not placed.
    penfii_core::admin::remove_member(&env.pool, env.owner.id, env.ws, admin.id)
        .await
        .unwrap();
    set_email(admin.id, "alias2@penfii.test").await;
    let accepted = accept(&admin, own_token).await;
    assert_eq!(accepted.status, StatusCode::CREATED, "{}", accepted.body);
    assert_eq!(accepted.body["team"]["status"], "SELF_PROMOTION");
    assert!(!in_team(&env, &team, admin.id).await);
    assert!(!administers(&env, &admin, &project).await);

    // Someone else's acceptance of the same admin's invitation still places a new person.
    let fresh = crate::common::call(
        &env.app,
        "POST",
        "/v1/invitations/accept",
        None,
        Some(json!({"token": fresh_token, "displayName": "fresh", "password": crate::common::MEMBER_PASSWORD})),
    )
    .await;
    assert_eq!(fresh.status, StatusCode::CREATED, "{}", fresh.body);
    assert_eq!(fresh.body["team"]["status"], "JOINED");
}
