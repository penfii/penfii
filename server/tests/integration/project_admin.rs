//! A Project keeps an admin who can act (FEA-054). "Active" is an `ACTIVE` member of the Workspace
//! whose account is not disabled. The last active direct admin cannot be demoted or removed from the
//! Project; leaving the Workspace or being disabled is never refused, and a Project left with no
//! active admin by any source is named an admin by a Workspace owner or admin, and only then.

use axum::http::StatusCode;
use penfii_core::CoreError;
use penfii_core::admin;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::common::{self, Person, Reply};

struct Env(common::Env);

impl std::ops::Deref for Env {
    type Target = common::Env;
    fn deref(&self) -> &common::Env {
        &self.0
    }
}

async fn setup(pool: PgPool) -> Env {
    Env(common::Env::new(pool).await)
}

impl Env {
    /// A Project made by the Workspace owner (who is its direct admin), optionally linked to Teams.
    async fn project(&self, teams: &[&str]) -> String {
        let reply = self
            .call(
                &self.owner,
                "POST",
                &format!("/v1/workspaces/{}/projects", self.ws),
                Some(json!({"name": "Alpha", "teamIds": teams})),
            )
            .await;
        assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
        reply.body["id"].as_str().unwrap().to_string()
    }

    fn grant_path(project: &str, who: Uuid) -> String {
        format!("/v1/projects/{project}/grants/{who}")
    }

    async fn grant(&self, by: &Person, project: &str, who: Uuid, role: &str) -> Reply {
        self.call(
            by,
            "PUT",
            &Self::grant_path(project, who),
            Some(json!({"role": role})),
        )
        .await
    }

    async fn ungrant(&self, by: &Person, project: &str, who: Uuid) -> Reply {
        self.call(by, "DELETE", &Self::grant_path(project, who), None)
            .await
    }

    async fn recover(&self, by: &Person, project: &str, who: Uuid) -> Reply {
        self.call(
            by,
            "POST",
            &format!("/v1/projects/{project}/admin-recovery"),
            Some(json!({"userId": who})),
        )
        .await
    }

    async fn reads(&self, who: &Person, project: &str) -> bool {
        self.call(who, "GET", &format!("/v1/projects/{project}/tickets"), None)
            .await
            .status
            == StatusCode::OK
    }

    async fn without_admin(&self, by: &Person) -> Vec<Value> {
        let reply = self
            .call(
                by,
                "GET",
                &format!("/v1/workspaces/{}/projects-without-admin", self.ws),
                None,
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
        reply.body["items"].as_array().unwrap().clone()
    }

    async fn impact(&self, by: &Person, who: Uuid) -> Vec<Value> {
        let reply = self
            .call(
                by,
                "GET",
                &format!(
                    "/v1/workspaces/{}/members/{who}/offboarding-impact",
                    self.ws
                ),
                None,
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
        reply.body["projects"].as_array().unwrap().clone()
    }

    /// A Team the owner manages with `members` in it, linked to nothing yet.
    async fn team(&self, members: &[&Person]) -> String {
        let team = self
            .call(
                &self.owner,
                "POST",
                &format!("/v1/workspaces/{}/teams", self.ws),
                Some(json!({"name": "Leads", "members": [{"userId": self.owner.id, "role": "MANAGER"}]})),
            )
            .await;
        assert_eq!(team.status, StatusCode::CREATED, "{}", team.body);
        let id = team.body["id"].as_str().unwrap().to_string();
        for member in members {
            let added = self
                .call(
                    &self.owner,
                    "PUT",
                    &format!(
                        "/v1/workspaces/{}/teams/{id}/members/{}",
                        self.ws, member.id
                    ),
                    Some(json!({"role": "MEMBER"})),
                )
                .await;
            assert_eq!(added.status, StatusCode::NO_CONTENT, "{}", added.body);
        }
        id
    }

    /// A Team run by `members` alone: the owner who has to make it is not in it afterwards.
    async fn team_of(&self, members: &[&Person]) -> String {
        let id = self.team(members).await;
        for member in members {
            let made = self
                .call(
                    &self.owner,
                    "PUT",
                    &format!(
                        "/v1/workspaces/{}/teams/{id}/members/{}",
                        self.ws, member.id
                    ),
                    Some(json!({"role": "MANAGER"})),
                )
                .await;
            assert_eq!(made.status, StatusCode::NO_CONTENT, "{}", made.body);
        }
        let left = self
            .call(
                &self.owner,
                "DELETE",
                &format!(
                    "/v1/workspaces/{}/teams/{id}/members/{}",
                    self.ws, self.owner.id
                ),
                None,
            )
            .await;
        assert_eq!(left.status, StatusCode::NO_CONTENT, "{}", left.body);
        id
    }

    async fn team_role(&self, by: &Person, project: &str, team: &str, role: Option<&str>) {
        let reply = self
            .call(
                by,
                "PUT",
                &format!("/v1/projects/{project}/teams/{team}/participation"),
                Some(json!({"role": role})),
            )
            .await;
        assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.body);
    }

    /// The person signed in again (disabling ended their sessions).
    async fn sign_in_again(&self, who: &Person) -> Person {
        let (status, cookie) = common::login(&self.app, &who.email, common::MEMBER_PASSWORD).await;
        assert_eq!(status, StatusCode::OK);
        Person {
            cookie: cookie.unwrap(),
            ..who.clone()
        }
    }

    async fn disable(&self, who: &Person) {
        admin::operator_set_account_disabled(&self.pool, &who.email, true)
            .await
            .unwrap();
    }

    async fn enable(&self, who: &Person) {
        admin::operator_set_account_disabled(&self.pool, &who.email, false)
            .await
            .unwrap();
    }

    async fn remove(&self, who: &Person) {
        admin::remove_member(&self.pool, self.owner.id, self.ws, who.id)
            .await
            .unwrap();
    }

    async fn audit(&self, action: &str) -> Vec<Value> {
        sqlx::query_scalar::<_, Value>(
            "SELECT data FROM penfii.audit_entries WHERE workspace_id = $1 AND action_code = $2
             ORDER BY occurred_at, id",
        )
        .bind(self.ws)
        .bind(action)
        .fetch_all(&self.pool)
        .await
        .unwrap()
    }
}

fn code(reply: &Reply) -> Option<&str> {
    reply.body["code"].as_str()
}

/// The Project of `lead`, the owner stepping aside, so `lead` is the only direct admin.
async fn led_by(env: &Env, lead: &Person) -> String {
    let project = env.project(&[]).await;
    assert!(
        env.grant(&env.owner, &project, lead.id, "ADMIN")
            .await
            .status
            .is_success()
    );
    assert_eq!(
        env.ungrant(&env.owner, &project, env.owner.id).await.status,
        StatusCode::NO_CONTENT
    );
    project
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn the_last_active_direct_admin_stays_however_it_is_asked(pool: PgPool) {
    let env = setup(pool).await;
    let lead = env.person("lead", "MEMBER").await;
    let project = led_by(&env, &lead).await;

    // Not by demoting, not by taking the grant away, not by themself, not by another admin's hand.
    for reply in [
        env.grant(&lead, &project, lead.id, "EDITOR").await,
        env.ungrant(&lead, &project, lead.id).await,
    ] {
        assert_eq!(reply.status, StatusCode::CONFLICT, "{}", reply.body);
        assert_eq!(code(&reply), Some("last_project_admin"));
    }
    // Giving the same role again, or any other grant, is not held up by it.
    assert!(
        env.grant(&lead, &project, lead.id, "ADMIN")
            .await
            .status
            .is_success()
    );
    let other = env.person("other", "MEMBER").await;
    assert!(
        env.grant(&lead, &project, other.id, "EDITOR")
            .await
            .status
            .is_success()
    );
    assert!(
        env.ungrant(&lead, &project, other.id)
            .await
            .status
            .is_success()
    );

    // With a second active direct admin the first may go.
    assert!(
        env.grant(&lead, &project, other.id, "ADMIN")
            .await
            .status
            .is_success()
    );
    assert_eq!(
        env.ungrant(&other, &project, lead.id).await.status,
        StatusCode::NO_CONTENT
    );
    // ... and now `other` is the last.
    let reply = env.grant(&other, &project, other.id, "VIEWER").await;
    assert_eq!(code(&reply), Some("last_project_admin"), "{}", reply.body);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_direct_admin_who_cannot_act_does_not_count_as_one_that_stays(pool: PgPool) {
    let env = setup(pool).await;
    let removed = env.person("removed", "MEMBER").await;
    let disabled = env.person("disabled", "MEMBER").await;
    let project = env.project(&[]).await;
    for who in [&removed, &disabled] {
        assert!(
            env.grant(&env.owner, &project, who.id, "ADMIN")
                .await
                .status
                .is_success()
        );
    }
    env.remove(&removed).await;
    env.disable(&disabled).await;

    // The owner is the only direct admin who can act, so the owner stays, whoever else holds a grant.
    for reply in [
        env.grant(&env.owner, &project, env.owner.id, "EDITOR")
            .await,
        env.ungrant(&env.owner, &project, env.owner.id).await,
    ] {
        assert_eq!(code(&reply), Some("last_project_admin"), "{}", reply.body);
    }
    // The grants of people who cannot act can still be tidied.
    assert_eq!(
        env.ungrant(&env.owner, &project, removed.id).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        env.ungrant(&env.owner, &project, disabled.id).await.status,
        StatusCode::NO_CONTENT
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn leaving_or_being_disabled_is_never_refused_and_leaves_the_project_without_an_admin(
    pool: PgPool,
) {
    let env = setup(pool).await;
    let lead = env.person("lead", "MEMBER").await;
    let gone = env.person("gone", "MEMBER").await;
    let paused = env.person("paused", "MEMBER").await;
    let lead_project = led_by(&env, &lead).await;
    let gone_project = led_by(&env, &gone).await;
    let paused_project = led_by(&env, &paused).await;
    assert!(env.without_admin(&env.owner).await.is_empty());

    env.remove(&gone).await;
    env.disable(&paused).await;
    let listed = env.without_admin(&env.owner).await;
    let ids: Vec<&str> = listed.iter().map(|p| p["id"].as_str().unwrap()).collect();
    assert_eq!(ids.len(), 2, "{listed:?}");
    assert!(ids.contains(&gone_project.as_str()) && ids.contains(&paused_project.as_str()));
    assert!(!ids.contains(&lead_project.as_str()));
    // Who held the role is shown, so the owner knows whom this was about.
    let gone_row = listed
        .iter()
        .find(|p| p["id"] == json!(gone_project))
        .unwrap();
    assert_eq!(gone_row["formerAdmins"][0]["displayName"], "gone");

    // Nobody can manage it, the Workspace owner included (a Project the owner has no role in).
    for (method, path, body) in [
        (
            "PUT",
            Env::grant_path(&gone_project, env.owner.id),
            Some(json!({"role": "ADMIN"})),
        ),
        ("GET", format!("/v1/projects/{gone_project}/trash"), None),
    ] {
        let reply = env.call(&env.owner, method, &path, body).await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{method} {path}");
    }
    // Only an owner or admin sees the list; a plain member does not.
    let reply = env
        .call(
            &lead,
            "GET",
            &format!("/v1/workspaces/{}/projects-without-admin", env.ws),
            None,
        )
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_workspace_owner_or_admin_names_an_admin_only_for_a_project_without_one(pool: PgPool) {
    let env = setup(pool).await;
    let wadmin = env.person("wadmin", "ADMIN").await;
    let lead = env.person("lead", "MEMBER").await;
    let member = env.person("member", "MEMBER").await;
    let project = led_by(&env, &lead).await;

    // With an admin the Project is not recovered this way, whoever asks.
    for by in [&env.owner, &wadmin] {
        let reply = env.recover(by, &project, member.id).await;
        assert_eq!(reply.status, StatusCode::CONFLICT, "{}", reply.body);
        assert_eq!(code(&reply), Some("project_has_admin"));
    }
    env.remove(&lead).await;

    // A plain member cannot, and a stranger to the Project learns nothing.
    assert_eq!(
        env.recover(&member, &project, member.id).await.status,
        StatusCode::FORBIDDEN
    );
    assert!(!env.reads(&wadmin, &project).await);

    // The Workspace admin names the member; that is all it does for them.
    let reply = env.recover(&wadmin, &project, member.id).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.body["newAccess"], true);
    assert!(
        env.reads(&member, &project).await,
        "the one named reads the Project"
    );
    assert!(
        !env.reads(&wadmin, &project).await,
        "the one who named does not"
    );
    assert!(!env.reads(&env.owner, &project).await);
    assert!(env.without_admin(&env.owner).await.is_empty());
    // It is audited, saying it opened the Project to someone new.
    let entries = env.audit("project.admin_recovered").await;
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert_eq!(entries[0]["userId"], json!(member.id));
    assert_eq!(entries[0]["newAccess"], true);
    assert_eq!(entries[0]["self"], false);
    // The Project has an admin again, so a second recovery is refused.
    let again = env.recover(&wadmin, &project, wadmin.id).await;
    assert_eq!(code(&again), Some("project_has_admin"));
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn the_one_who_recovers_may_name_themself_and_the_warning_is_in_the_audit(pool: PgPool) {
    let env = setup(pool).await;
    let lead = env.person("lead", "MEMBER").await;
    let project = led_by(&env, &lead).await;
    env.remove(&lead).await;
    assert!(!env.reads(&env.owner, &project).await);
    let reply = env.recover(&env.owner, &project, env.owner.id).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.body["newAccess"], true);
    assert!(
        env.reads(&env.owner, &project).await,
        "naming themself is how they get in"
    );
    let entries = env.audit("project.admin_recovered").await;
    assert_eq!(entries[0]["self"], true);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn the_named_person_must_be_able_to_act_and_a_person_who_already_took_part_is_not_new_access(
    pool: PgPool,
) {
    let env = setup(pool).await;
    let lead = env.person("lead", "MEMBER").await;
    let editor = env.person("editor", "MEMBER").await;
    let off = env.person("off", "MEMBER").await;
    let gone = env.person("gone", "MEMBER").await;
    let project = led_by(&env, &lead).await;
    assert!(
        env.grant(&lead, &project, editor.id, "EDITOR")
            .await
            .status
            .is_success()
    );
    env.remove(&lead).await;

    env.disable(&off).await;
    env.remove(&gone).await;
    let stranger = Person {
        id: Uuid::new_v4(),
        ..editor.clone()
    };
    for who in [off.id, gone.id, stranger.id] {
        let reply = env.recover(&env.owner, &project, who).await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{}", reply.body);
    }
    // A participant who is made admin was not given access by it.
    let reply = env.recover(&env.owner, &project, editor.id).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.body["newAccess"], false);
    assert_eq!(
        env.audit("project.admin_recovered").await[0]["previousRole"],
        "EDITOR"
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn an_admin_through_a_team_or_a_group_is_an_active_admin_and_a_disabled_one_is_not(
    pool: PgPool,
) {
    let env = setup(pool.clone()).await;
    let lead = env.person("lead", "MEMBER").await;
    let by_team = env.person("byteam", "MEMBER").await;
    let by_group = env.person("bygroup", "MEMBER").await;
    let project = led_by(&env, &lead).await;
    let team = env.team_of(&[&by_team]).await;
    let linked = env
        .call(
            &lead,
            "PUT",
            &format!("/v1/projects/{project}/teams/{team}"),
            None,
        )
        .await;
    assert!(linked.status.is_success(), "{}", linked.body);
    env.team_role(&lead, &project, &team, Some("ADMIN")).await;
    env.remove(&lead).await;

    // The Team's admin keeps the Project in hand, so nobody recovers it from outside.
    assert!(env.without_admin(&env.owner).await.is_empty());
    assert_eq!(
        code(&env.recover(&env.owner, &project, env.owner.id).await),
        Some("project_has_admin")
    );
    assert!(env.reads(&by_team, &project).await);
    // They are not a direct admin: the grant of the person who left can be tidied away, and they can
    // name themself a direct admin, without the rule about the last direct admin standing in the way.
    assert_eq!(
        env.ungrant(&by_team, &project, lead.id).await.status,
        StatusCode::NO_CONTENT
    );
    assert!(
        env.grant(&by_team, &project, by_team.id, "ADMIN")
            .await
            .status
            .is_success()
    );
    assert_eq!(
        env.ungrant(&by_team, &project, by_team.id).await.status,
        StatusCode::CONFLICT
    );
    assert!(env.ungrant(&by_team, &project, by_team.id).await.body["code"] == "last_project_admin");

    // Their account is disabled: the Team no longer counts, and the Project is without an admin.
    env.disable(&by_team).await;
    env.remove_grant_rows(&project).await;
    assert_eq!(env.without_admin(&env.owner).await.len(), 1);
    env.enable(&by_team).await;
    let by_team = env.sign_in_again(&by_team).await;
    assert!(env.without_admin(&env.owner).await.is_empty());

    // A Group with the admin role counts the same way, by its own members or the Team behind it.
    env.team_role(&by_team, &project, &team, Some("EDITOR"))
        .await;
    assert_eq!(env.without_admin(&env.owner).await.len(), 1);
    let group = Uuid::new_v4();
    sqlx::query("INSERT INTO penfii.groups (id, workspace_id, name) VALUES ($1, $2, 'Leads')")
        .bind(group)
        .bind(env.ws)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO penfii.group_members (group_id, user_id) VALUES ($1, $2)")
        .bind(group)
        .bind(by_group.id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO penfii.project_group_grants (project_id, group_id, role) VALUES ($1, $2, 'ADMIN')")
        .bind(Uuid::parse_str(&project).unwrap())
        .bind(group)
        .execute(&pool)
        .await
        .unwrap();
    assert!(env.without_admin(&env.owner).await.is_empty());
    assert!(env.reads(&by_group, &project).await);
    env.disable(&by_group).await;
    assert_eq!(env.without_admin(&env.owner).await.len(), 1);
}

impl Env {
    /// Drops the direct admin grants of a Project, which the Team test does not want in the way.
    async fn remove_grant_rows(&self, project: &str) {
        sqlx::query("DELETE FROM penfii.project_user_grants WHERE project_id = $1")
            .bind(Uuid::parse_str(project).unwrap())
            .execute(&self.pool)
            .await
            .unwrap();
    }
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn enabling_an_account_brings_its_admin_role_back_next_to_the_one_named_meanwhile(
    pool: PgPool,
) {
    let env = setup(pool.clone()).await;
    let lead = env.person("lead", "MEMBER").await;
    let project = led_by(&env, &lead).await;
    env.disable(&lead).await;
    assert_eq!(env.without_admin(&env.owner).await.len(), 1);
    let named = env.recover(&env.owner, &project, env.owner.id).await;
    assert_eq!(named.status, StatusCode::OK, "{}", named.body);

    // Enabled again, the person is an admin as before; nothing is cleaned up and nobody is demoted.
    env.enable(&lead).await;
    let lead = env.sign_in_again(&lead).await;
    assert!(env.reads(&lead, &project).await);
    assert!(env.reads(&env.owner, &project).await);
    let admins: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.project_user_grants WHERE project_id = $1 AND role = 'ADMIN'",
    )
    .bind(Uuid::parse_str(&project).unwrap())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(admins, 2);
    assert!(env.without_admin(&env.owner).await.is_empty());
    // And with two, either may go.
    assert_eq!(
        env.ungrant(&env.owner, &project, lead.id).await.status,
        StatusCode::NO_CONTENT
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn the_warning_before_leaving_names_the_projects_that_lose_their_last_admin(pool: PgPool) {
    let env = setup(pool.clone()).await;
    let lead = env.person("lead", "MEMBER").await;
    let second = env.person("second", "MEMBER").await;
    let team_admin = env.person("teamadmin", "MEMBER").await;
    let sole = led_by(&env, &lead).await; // lead alone: nobody is left
    let with_team = led_by(&env, &lead).await; // lead alone directly, a Team admin stays
    let shared = led_by(&env, &lead).await; // another direct admin stays
    let team = env.team_of(&[&team_admin]).await;
    let linked = env
        .call(
            &lead,
            "PUT",
            &format!("/v1/projects/{with_team}/teams/{team}"),
            None,
        )
        .await;
    assert!(linked.status.is_success(), "{}", linked.body);
    env.team_role(&lead, &with_team, &team, Some("ADMIN")).await;
    assert!(
        env.grant(&lead, &shared, second.id, "ADMIN")
            .await
            .status
            .is_success()
    );

    let impact = env.impact(&env.owner, lead.id).await;
    let kind_of = |project: &str| {
        impact
            .iter()
            .find(|row| row["projectId"] == json!(project))
            .map(|row| row["kind"].as_str().unwrap().to_string())
    };
    assert_eq!(kind_of(&sole).as_deref(), Some("NO_ADMIN"), "{impact:?}");
    assert_eq!(
        kind_of(&with_team).as_deref(),
        Some("NO_DIRECT_ADMIN"),
        "{impact:?}"
    );
    assert_eq!(kind_of(&shared), None, "another direct admin stays");
    // Someone who is only a Team admin and the last one standing is warned about too.
    let impact = env.impact(&env.owner, team_admin.id).await;
    assert!(
        impact.is_empty(),
        "the Team admin is not the last admin of a Project the lead leads"
    );

    // The warning is for owners and admins, about active members.
    assert_eq!(
        env.call(
            &lead,
            "GET",
            &format!(
                "/v1/workspaces/{}/members/{}/offboarding-impact",
                env.ws, lead.id
            ),
            None
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        env.call(
            &env.owner,
            "GET",
            &format!(
                "/v1/workspaces/{}/members/{}/offboarding-impact",
                env.ws,
                Uuid::new_v4()
            ),
            None
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );

    // And it only warns: removing and disabling go through, and the operator's disable reports the same.
    let change = admin::operator_set_account_disabled(&pool, &lead.email, true)
        .await
        .unwrap();
    assert_eq!(change.impacts.len(), 1);
    let kinds: Vec<(&str, &str)> = change.impacts[0]
        .projects
        .iter()
        .map(|p| (p.name.as_str(), p.kind.as_str()))
        .collect();
    assert_eq!(kinds.len(), 2, "{kinds:?}");
    assert!(
        change.impacts[0]
            .projects
            .iter()
            .any(|p| p.kind == "NO_ADMIN")
    );
    assert!(
        change.impacts[0]
            .projects
            .iter()
            .any(|p| p.kind == "NO_DIRECT_ADMIN")
    );
    // Disabling again says nothing: the person no longer counts as an admin.
    let again = admin::operator_set_account_disabled(&pool, &lead.email, true)
        .await
        .unwrap();
    assert!(again.impacts.is_empty());
    env.enable(&lead).await;
    env.remove(&lead).await;
    // Only the Project nobody else runs is left without an admin: the Team's admin and the second
    // direct admin keep theirs.
    let left = env.without_admin(&env.owner).await;
    assert_eq!(left.len(), 1, "{left:?}");
    assert_eq!(left[0]["id"], json!(sole));
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn two_admins_giving_up_the_role_at_once_leave_the_project_one(pool: PgPool) {
    let env = setup(pool.clone()).await;
    for round in 0..6 {
        let a = env.person(&format!("a{round}"), "MEMBER").await;
        let b = env.person(&format!("b{round}"), "MEMBER").await;
        let project = env.project(&[]).await;
        for who in [&a, &b] {
            assert!(
                env.grant(&env.owner, &project, who.id, "ADMIN")
                    .await
                    .status
                    .is_success()
            );
        }
        assert!(
            env.ungrant(&env.owner, &project, env.owner.id)
                .await
                .status
                .is_success()
        );
        let step_aside = |who: Person| {
            let (app, project) = (env.app.clone(), project.clone());
            tokio::spawn(async move {
                common::call(
                    &app,
                    "PUT",
                    &Env::grant_path(&project, who.id),
                    Some(&who.cookie),
                    Some(json!({"role": "VIEWER"})),
                )
                .await
                .status
            })
        };
        let (x, y) = (step_aside(a.clone()), step_aside(b.clone()));
        let results = [x.await.unwrap(), y.await.unwrap()];
        assert_eq!(
            results.iter().filter(|s| s.is_success()).count(),
            1,
            "{results:?}"
        );
        assert!(results.contains(&StatusCode::CONFLICT), "{results:?}");
        let admins: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM penfii.project_user_grants WHERE project_id = $1 AND role = 'ADMIN'",
        )
        .bind(Uuid::parse_str(&project).unwrap())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(admins, 1);
    }
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn two_recoveries_at_once_name_one_admin(pool: PgPool) {
    let env = setup(pool.clone()).await;
    let wadmin = env.person("wadmin", "ADMIN").await;
    for round in 0..6 {
        let lead = env.person(&format!("lead{round}"), "MEMBER").await;
        let x = env.person(&format!("x{round}"), "MEMBER").await;
        let y = env.person(&format!("y{round}"), "MEMBER").await;
        let project = led_by(&env, &lead).await;
        env.remove(&lead).await;
        let recover = |by: Person, who: Uuid| {
            let (app, project) = (env.app.clone(), project.clone());
            tokio::spawn(async move {
                common::call(
                    &app,
                    "POST",
                    &format!("/v1/projects/{project}/admin-recovery"),
                    Some(&by.cookie),
                    Some(json!({"userId": who})),
                )
                .await
                .status
            })
        };
        let (first, second) = (
            recover(env.owner.clone(), x.id),
            recover(wadmin.clone(), y.id),
        );
        let results = [first.await.unwrap(), second.await.unwrap()];
        assert_eq!(
            results.iter().filter(|s| s.is_success()).count(),
            1,
            "{results:?}"
        );
        assert!(results.contains(&StatusCode::CONFLICT), "{results:?}");
        assert_eq!(env.audit("project.admin_recovered").await.len(), round + 1);
    }
}

// The same meaning of "active" wherever a Workspace, a Team or a channel keeps someone to run it.

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_disabled_owner_does_not_keep_the_workspace_its_owner(pool: PgPool) {
    let env = setup(pool.clone()).await;
    let second = env.person("second", "MEMBER").await;
    admin::operator_set_member_role(&pool, env.ws, &second.email, "OWNER")
        .await
        .unwrap();
    env.disable(&second).await;

    // The only owner who can act stays an owner, however the disabled one is counted otherwise.
    let last = admin::operator_set_member_role(&pool, env.ws, common::OWNER_EMAIL, "ADMIN")
        .await
        .unwrap_err();
    assert!(
        matches!(
            last,
            CoreError::Conflict {
                code: "last_owner",
                ..
            }
        ),
        "{last:?}"
    );
    // The disabled owner can be demoted, and removed, while an active one remains.
    admin::operator_set_member_role(&pool, env.ws, &second.email, "ADMIN")
        .await
        .unwrap();
    env.enable(&second).await;
    admin::operator_set_member_role(&pool, env.ws, &second.email, "OWNER")
        .await
        .unwrap();
    env.disable(&second).await;
    env.remove(&second).await;
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_disabled_manager_does_not_keep_a_team_or_a_channel_run(pool: PgPool) {
    let env = setup(pool.clone()).await;
    let other = env.person("other", "MEMBER").await;

    // A Team with the owner and `other` as managers.
    let team = env.team(&[&other]).await;
    let promoted = env
        .call(
            &env.owner,
            "PUT",
            &format!(
                "/v1/workspaces/{}/teams/{team}/members/{}",
                env.ws, other.id
            ),
            Some(json!({"role": "MANAGER"})),
        )
        .await;
    assert_eq!(promoted.status, StatusCode::NO_CONTENT, "{}", promoted.body);
    let step_down = || async {
        env.call(
            &env.owner,
            "PUT",
            &format!(
                "/v1/workspaces/{}/teams/{team}/members/{}",
                env.ws, env.owner.id
            ),
            Some(json!({"role": "MEMBER"})),
        )
        .await
    };
    env.disable(&other).await;
    let refused = step_down().await;
    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.body);
    assert_eq!(refused.body["code"], "last_team_manager");
    env.enable(&other).await;
    assert_eq!(step_down().await.status, StatusCode::NO_CONTENT);

    // A private channel the owner runs with `other` as its second manager.
    let channel = env
        .call(
            &env.owner,
            "POST",
            &format!("/v1/workspaces/{}/channels", env.ws),
            Some(json!({"name": "leads", "visibility": "PRIVATE"})),
        )
        .await
        .body["id"]
        .as_str()
        .unwrap()
        .to_string();
    let member = format!("/v1/channels/{channel}/members/{}", other.id);
    assert_eq!(
        env.call(&env.owner, "PUT", &member, None).await.status,
        StatusCode::OK
    );
    let made = env
        .call(
            &env.owner,
            "PATCH",
            &member,
            Some(json!({"role": "MANAGER"})),
        )
        .await;
    assert_eq!(made.status, StatusCode::OK, "{}", made.body);
    env.disable(&other).await;
    let leave = || async {
        env.call(
            &env.owner,
            "DELETE",
            &format!("/v1/channels/{channel}/members/{}", env.owner.id),
            None,
        )
        .await
    };
    let stuck = leave().await;
    assert_eq!(stuck.status, StatusCode::CONFLICT, "{}", stuck.body);
    assert_eq!(stuck.body["code"], "last_manager");
    env.enable(&other).await;
    assert_eq!(leave().await.status, StatusCode::NO_CONTENT);
}
