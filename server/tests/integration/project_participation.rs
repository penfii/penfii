//! Project participation and the channels of the Project (FEA-055, FEA-060, FEA-061).
//!
//! * A `channel_members` row of a Project's channel counts only while the person takes part in the
//!   Project (and, for a Team's channel, in the Team): without that it grants no reading, no writing
//!   and no role (FEA-060). The rows these tests plant by hand stand for the ones an older version
//!   left behind.
//! * Every change that ends a participation takes the person's memberships of the Project's channels
//!   with it, in the same transaction, and nothing comes back with the participation (FEA-061). A
//!   person who still takes part some other way keeps theirs.

use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::common::{self, Person, doc};

struct Env {
    env: common::Env,
    project: String,
    /// The Project's private and public channel (the person has a row in both).
    private: String,
    public: String,
}

impl std::ops::Deref for Env {
    type Target = common::Env;

    fn deref(&self) -> &common::Env {
        &self.env
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
struct Standing {
    row: bool,
    read: bool,
    write: bool,
}

const fn standing(row: bool, read: bool, write: bool) -> Standing {
    Standing { row, read, write }
}

const FULL: Standing = standing(true, true, true);
/// A row that exists and grants nothing.
const DEAD_ROW: Standing = standing(true, false, false);
const OPEN_WITHOUT_ROW: Standing = standing(false, true, true);
const SHUT: Standing = standing(false, false, false);

impl Env {
    async fn ok(&self, who: &Person, method: &str, path: &str, body: Option<Value>) -> Value {
        let reply = self.call(who, method, path, body).await;
        assert!(
            reply.status.is_success(),
            "{method} {path}: {} {}",
            reply.status,
            reply.body
        );
        reply.body
    }

    async fn channel(&self, by: &Person, visibility: &str, project: &str, name: &str) -> String {
        let created = self
            .call(
                by,
                "POST",
                &format!("/v1/workspaces/{}/channels", self.ws),
                Some(json!({"name": name, "visibility": visibility, "projectId": project})),
            )
            .await;
        assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
        created.body["id"].as_str().unwrap().to_string()
    }

    async fn team(&self, members: &[&Person]) -> String {
        let team = self
            .ok(
                &self.owner,
                "POST",
                &format!("/v1/workspaces/{}/teams", self.ws),
                Some(
                    json!({"name": "Crew", "members": [{"userId": self.owner.id, "role": "MANAGER"}]}),
                ),
            )
            .await;
        let id = team["id"].as_str().unwrap().to_string();
        for member in members {
            self.ok(
                &self.owner,
                "PUT",
                &format!(
                    "/v1/workspaces/{}/teams/{id}/members/{}",
                    self.ws, member.id
                ),
                Some(json!({"role": "MEMBER"})),
            )
            .await;
        }
        id
    }

    /// Links the Team to the Project and lets its members in as editors.
    async fn team_in(&self, team: &str) {
        self.ok(
            &self.owner,
            "PUT",
            &format!("/v1/projects/{}/teams/{team}", self.project),
            None,
        )
        .await;
        self.ok(
            &self.owner,
            "PUT",
            &format!("/v1/projects/{}/teams/{team}/participation", self.project),
            Some(json!({"role": "EDITOR"})),
        )
        .await;
    }

    async fn grant(&self, who: &Person, role: &str) {
        self.ok(
            &self.owner,
            "PUT",
            &format!("/v1/projects/{}/grants/{}", self.project, who.id),
            Some(json!({"role": role})),
        )
        .await;
    }

    async fn ungrant(&self, who: &Person) {
        self.ok(
            &self.owner,
            "DELETE",
            &format!("/v1/projects/{}/grants/{}", self.project, who.id),
            None,
        )
        .await;
    }

    /// The person is put into the private channel and joins the public one.
    async fn rows_for(&self, who: &Person) {
        self.ok(
            &self.owner,
            "PUT",
            &format!("/v1/channels/{}/members/{}", self.private, who.id),
            None,
        )
        .await;
        self.ok(
            who,
            "POST",
            &format!("/v1/channels/{}/join", self.public),
            None,
        )
        .await;
    }

    async fn row(&self, who: &Person, channel: &str) -> Option<String> {
        sqlx::query_scalar(
            "SELECT role FROM penfii.channel_members WHERE channel_id = $1 AND user_id = $2",
        )
        .bind(channel.parse::<Uuid>().unwrap())
        .bind(who.id)
        .fetch_optional(&self.pool)
        .await
        .unwrap()
    }

    /// A row as an older version left it behind.
    async fn plant(&self, who: &Person, channel: &str, role: &str) {
        sqlx::query(
            "INSERT INTO penfii.channel_members (channel_id, user_id, role) VALUES ($1, $2, $3)
             ON CONFLICT (channel_id, user_id) DO UPDATE SET role = EXCLUDED.role",
        )
        .bind(channel.parse::<Uuid>().unwrap())
        .bind(who.id)
        .bind(role)
        .execute(&self.pool)
        .await
        .unwrap();
    }

    async fn standing(&self, who: &Person, channel: &str) -> Standing {
        let read = self
            .call(
                who,
                "GET",
                &format!("/v1/conversations/{channel}/messages"),
                None,
            )
            .await
            .status
            == StatusCode::OK;
        let write = self
            .call(
                who,
                "POST",
                &format!("/v1/conversations/{channel}/messages"),
                Some(json!({"body": doc("hello"), "bodySchemaVersion": 1, "threadRootId": null, "replyToId": null})),
            )
            .await
            .status
            .is_success();
        Standing {
            row: self.row(who, channel).await.is_some(),
            read,
            write,
        }
    }

    async fn pair(&self, who: &Person) -> [Standing; 2] {
        [
            self.standing(who, &self.private).await,
            self.standing(who, &self.public).await,
        ]
    }

    async fn space_lists(&self, who: &Person, channel: &str) -> bool {
        let space = self
            .ok(
                who,
                "GET",
                &format!("/v1/workspaces/{}/space", self.ws),
                None,
            )
            .await;
        space.to_string().contains(channel)
    }

    async fn without_manager(&self) -> Vec<String> {
        let reply = self
            .ok(
                &self.owner,
                "GET",
                &format!("/v1/workspaces/{}/channels-without-manager", self.ws),
                None,
            )
            .await;
        reply["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["id"].as_str().unwrap().to_string())
            .collect()
    }
}

/// A Project with a private and a public channel, and `pat` an editor of it with a row in both.
async fn setup(pool: PgPool) -> (Env, Person) {
    let env = common::Env::new(pool).await;
    let pat = env.person("pat", "MEMBER").await;
    let mut env = Env {
        env,
        project: String::new(),
        private: String::new(),
        public: String::new(),
    };
    let created = env
        .ok(
            &env.owner,
            "POST",
            &format!("/v1/workspaces/{}/projects", env.ws),
            Some(json!({"name": "Alpha", "teamIds": []})),
        )
        .await;
    let project = created["id"].as_str().unwrap().to_string();
    env.project = project.clone();
    env.grant(&pat, "EDITOR").await;
    env.private = env
        .channel(&env.owner, "PRIVATE", &project, "p-private")
        .await;
    env.public = env
        .channel(&env.owner, "PUBLIC", &project, "p-public")
        .await;
    env.rows_for(&pat).await;
    assert_eq!(env.pair(&pat).await, [FULL, FULL]);
    (env, pat)
}

// ---- FEA-060: a row alone grants nothing ------------------------------------------------------

/// Rows that outlived the participation (planted: nothing the application does leaves them any
/// more) are not a way in: not reading, writing, the Space list, the member list, nor management.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_row_without_the_participation_grants_nothing(pool: PgPool) {
    let (env, pat) = setup(pool).await;
    env.ungrant(&pat).await;
    env.plant(&pat, &env.private, "MANAGER").await;
    env.plant(&pat, &env.public, "MEMBER").await;
    assert_eq!(env.pair(&pat).await, [DEAD_ROW, DEAD_ROW]);
    assert!(!env.space_lists(&pat, &env.private).await);
    assert!(!env.space_lists(&pat, &env.public).await);

    // The member list does not show them and they cannot read it.
    let theirs = env
        .call(
            &pat,
            "GET",
            &format!("/v1/channels/{}/members", env.private),
            None,
        )
        .await;
    assert_eq!(theirs.status, StatusCode::NOT_FOUND);
    let listed = env
        .ok(
            &env.owner,
            "GET",
            &format!("/v1/channels/{}/members", env.private),
            None,
        )
        .await;
    assert!(
        !listed.to_string().contains(&pat.id.to_string()),
        "{listed}"
    );

    // The Manager row is no Manager: settings and webhooks are closed, nobody can be put in.
    let describe = json!({"description": "mine now"});
    let patched = env
        .call(
            &pat,
            "PATCH",
            &format!("/v1/channels/{}", env.private),
            Some(describe),
        )
        .await;
    assert_eq!(patched.status, StatusCode::NOT_FOUND, "{}", patched.body);
    let webhooks = env
        .call(
            &pat,
            "GET",
            &format!("/v1/channels/{}/webhooks", env.private),
            None,
        )
        .await;
    assert_eq!(webhooks.status, StatusCode::NOT_FOUND, "{}", webhooks.body);
    let adds = env
        .call(
            &pat,
            "PUT",
            &format!("/v1/channels/{}/members/{}", env.private, env.owner.id),
            None,
        )
        .await;
    assert_eq!(adds.status, StatusCode::NOT_FOUND, "{}", adds.body);
}

/// A Manager row of someone who lost the Project is no manager: the channel counts as having none
/// and is offered for recovery, and the last-manager guard does not count them either.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_manager_row_without_the_participation_is_no_manager(pool: PgPool) {
    let (env, pat) = setup(pool).await;
    env.grant(&pat, "ADMIN").await;
    let own = env.channel(&pat, "PRIVATE", &env.project, "pat-runs").await;
    assert!(!env.without_manager().await.contains(&own));

    // Losing the Project takes the row with it, and the channel is recoverable at once.
    env.ungrant(&pat).await;
    assert_eq!(env.row(&pat, &own).await, None);
    assert!(env.without_manager().await.contains(&own));

    // The same with a row planted afterwards.
    env.plant(&pat, &own, "MANAGER").await;
    assert!(
        env.without_manager().await.contains(&own),
        "a Manager row without the Project does not count as a manager"
    );
}

// ---- FEA-061: the ways of losing the participation --------------------------------------------

#[derive(Clone, Copy, Debug)]
enum Way {
    GrantRemoved,
    TeamUnlinked,
    TeamParticipationEnded,
    TeamMemberRemoved,
}

/// `pat` takes part only through the way under test, has rows in both channels and manages the
/// private one besides the owner.
async fn only_through(pool: PgPool, way: Way) -> (Env, Person, Option<String>) {
    let env = common::Env::new(pool).await;
    let pat = env.person("pat", "MEMBER").await;
    let mut env = Env {
        env,
        project: String::new(),
        private: String::new(),
        public: String::new(),
    };
    let created = env
        .ok(
            &env.owner,
            "POST",
            &format!("/v1/workspaces/{}/projects", env.ws),
            Some(json!({"name": "Alpha", "teamIds": []})),
        )
        .await;
    let project = created["id"].as_str().unwrap().to_string();
    env.project = project.clone();
    let team = match way {
        Way::GrantRemoved => {
            env.grant(&pat, "EDITOR").await;
            None
        }
        _ => {
            let team = env.team(&[&pat]).await;
            env.team_in(&team).await;
            Some(team)
        }
    };
    env.private = env
        .channel(&env.owner, "PRIVATE", &project, "p-private")
        .await;
    env.public = env
        .channel(&env.owner, "PUBLIC", &project, "p-public")
        .await;
    env.rows_for(&pat).await;
    env.ok(
        &env.owner,
        "PATCH",
        &format!("/v1/channels/{}/members/{}", env.private, pat.id),
        Some(json!({"role": "MANAGER"})),
    )
    .await;
    assert_eq!(env.pair(&pat).await, [FULL, FULL]);
    (env, pat, team)
}

async fn lose(env: &Env, pat: &Person, team: &Option<String>, way: Way) {
    match way {
        Way::GrantRemoved => env.ungrant(pat).await,
        Way::TeamUnlinked => {
            env.ok(
                &env.owner,
                "DELETE",
                &format!(
                    "/v1/projects/{}/teams/{}",
                    env.project,
                    team.as_ref().unwrap()
                ),
                None,
            )
            .await;
        }
        Way::TeamParticipationEnded => {
            env.ok(
                &env.owner,
                "PUT",
                &format!(
                    "/v1/projects/{}/teams/{}/participation",
                    env.project,
                    team.as_ref().unwrap()
                ),
                Some(json!({"role": null})),
            )
            .await;
        }
        Way::TeamMemberRemoved => {
            env.ok(
                &env.owner,
                "DELETE",
                &format!(
                    "/v1/workspaces/{}/teams/{}/members/{}",
                    env.ws,
                    team.as_ref().unwrap(),
                    pat.id
                ),
                None,
            )
            .await;
        }
    }
}

async fn regain(env: &Env, pat: &Person, team: &Option<String>, way: Way) {
    match way {
        Way::GrantRemoved => env.grant(pat, "EDITOR").await,
        Way::TeamUnlinked => env.team_in(team.as_ref().unwrap()).await,
        Way::TeamParticipationEnded => {
            env.ok(
                &env.owner,
                "PUT",
                &format!(
                    "/v1/projects/{}/teams/{}/participation",
                    env.project,
                    team.as_ref().unwrap()
                ),
                Some(json!({"role": "EDITOR"})),
            )
            .await;
        }
        Way::TeamMemberRemoved => {
            env.ok(
                &env.owner,
                "PUT",
                &format!(
                    "/v1/workspaces/{}/teams/{}/members/{}",
                    env.ws,
                    team.as_ref().unwrap(),
                    pat.id
                ),
                Some(json!({"role": "MEMBER"})),
            )
            .await;
        }
    }
}

async fn each_way_takes_the_memberships_and_restores_none(pool: PgPool, way: Way) {
    let (env, pat, team) = only_through(pool, way).await;
    // A message of theirs and a personal setting that hang on the channel, not on the membership.
    let sent = env
        .ok(
            &pat,
            "POST",
            &format!("/v1/conversations/{}/messages", env.private),
            Some(json!({"body": doc("kept"), "bodySchemaVersion": 1, "threadRootId": null, "replyToId": null})),
        )
        .await;
    let message_id = sent["id"].as_str().unwrap_or_default().to_string();
    sqlx::query("INSERT INTO penfii.channel_mutes (user_id, channel_id) VALUES ($1, $2)")
        .bind(pat.id)
        .bind(env.private.parse::<Uuid>().unwrap())
        .execute(&env.pool)
        .await
        .unwrap();

    lose(&env, &pat, &team, way).await;

    assert_eq!(
        env.pair(&pat).await,
        [SHUT, SHUT],
        "{way:?}: closed, and the rows are gone"
    );
    assert!(!env.space_lists(&pat, &env.private).await, "{way:?}");
    assert!(!env.space_lists(&pat, &env.public).await, "{way:?}");
    // The Manager role went with the row: the owner's is the one that remains.
    assert_eq!(env.row(&pat, &env.private).await, None);
    let members = env
        .ok(
            &env.owner,
            "GET",
            &format!("/v1/channels/{}/members", env.private),
            None,
        )
        .await;
    assert!(
        !members.to_string().contains(&pat.id.to_string()),
        "{way:?}"
    );
    // What does not hang on the membership stays.
    let messages: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.messages WHERE conversation_id = $1 AND author_id = $2",
    )
    .bind(env.private.parse::<Uuid>().unwrap())
    .bind(pat.id)
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert!(messages >= 1, "{way:?}: the message stays ({message_id})");
    let mutes: i64 =
        sqlx::query_scalar("SELECT count(*) FROM penfii.channel_mutes WHERE user_id = $1")
            .bind(pat.id)
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert_eq!(mutes, 1, "{way:?}: the mute stays");
    // It is announced, and the audit entry counts it.
    let removed: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.domain_events WHERE event_kind = 'channel.member_removed'
           AND facts->>'userId' = $1 AND facts->>'reason' = 'project_participation_lost'",
    )
    .bind(pat.id.to_string())
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(removed, 2, "{way:?}: one event for each channel");
    let counted: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.audit_entries
         WHERE data->'clearedChannels'->>'private' = '1' AND data->'clearedChannels'->>'public' = '1'",
    )
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(
        counted, 1,
        "{way:?}: the audit entry names how many were cleared"
    );

    // Taking part again brings nothing back: the private channel is shut, the public one opens by
    // the Project alone, and nobody is Manager by an old row.
    regain(&env, &pat, &team, way).await;
    assert_eq!(
        env.pair(&pat).await,
        [SHUT, OPEN_WITHOUT_ROW],
        "{way:?}: nothing is restored"
    );
    assert_eq!(env.row(&pat, &env.private).await, None);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn removing_the_direct_role_takes_the_memberships(pool: PgPool) {
    each_way_takes_the_memberships_and_restores_none(pool, Way::GrantRemoved).await;
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn unlinking_the_team_takes_the_memberships(pool: PgPool) {
    each_way_takes_the_memberships_and_restores_none(pool, Way::TeamUnlinked).await;
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn ending_the_teams_participation_takes_the_memberships(pool: PgPool) {
    each_way_takes_the_memberships_and_restores_none(pool, Way::TeamParticipationEnded).await;
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn leaving_the_team_takes_the_memberships(pool: PgPool) {
    each_way_takes_the_memberships_and_restores_none(pool, Way::TeamMemberRemoved).await;
}

/// Participation that stays through another way keeps the memberships.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn another_way_in_keeps_the_memberships(pool: PgPool) {
    let (env, pat) = setup(pool).await;
    let team = env.team(&[&pat]).await;
    env.team_in(&team).await;
    // The direct role goes; the Team still lets pat in.
    env.ungrant(&pat).await;
    assert_eq!(env.pair(&pat).await, [FULL, FULL]);
    // The Team's role goes too: now nothing does.
    env.ok(
        &env.owner,
        "PUT",
        &format!("/v1/projects/{}/teams/{team}/participation", env.project),
        Some(json!({"role": null})),
    )
    .await;
    assert_eq!(env.pair(&pat).await, [SHUT, SHUT]);
}

/// A channel narrowed to a Team is lost with the Team alone, even though the direct role stays.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_teams_channel_is_lost_with_the_team(pool: PgPool) {
    let (env, pat) = setup(pool).await;
    let team = env.team(&[&pat]).await;
    env.team_in(&team).await;
    let narrowed = env
        .call(
            &env.owner,
            "POST",
            &format!("/v1/workspaces/{}/channels", env.ws),
            Some(json!({"name": "crew-only", "visibility": "PRIVATE", "projectId": env.project, "teamId": team})),
        )
        .await;
    assert_eq!(narrowed.status, StatusCode::CREATED, "{}", narrowed.body);
    let narrowed = narrowed.body["id"].as_str().unwrap().to_string();
    env.ok(
        &env.owner,
        "PUT",
        &format!("/v1/channels/{narrowed}/members/{}", pat.id),
        None,
    )
    .await;
    assert_eq!(env.standing(&pat, &narrowed).await, FULL);
    env.ok(
        &env.owner,
        "DELETE",
        &format!("/v1/workspaces/{}/teams/{team}/members/{}", env.ws, pat.id),
        None,
    )
    .await;
    assert_eq!(env.standing(&pat, &narrowed).await, SHUT);
    // The direct role is still there: the Project channels are not touched.
    assert_eq!(env.pair(&pat).await, [FULL, FULL]);
}

/// The creator of a Team's channel has to take part through the Team: being the Project's admin does not
/// get around it, and no creator exception exists afterwards either.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn the_creator_of_a_teams_channel_must_be_in_the_team(pool: PgPool) {
    let (env, pat) = setup(pool).await;
    let team = env.team(&[&pat]).await;
    env.team_in(&team).await;
    // The owner made the Team, so they start in it; pat leads it instead and the owner goes.
    env.ok(
        &env.owner,
        "PUT",
        &format!("/v1/workspaces/{}/teams/{team}/members/{}", env.ws, pat.id),
        Some(json!({"role": "MANAGER"})),
    )
    .await;
    env.ok(
        &env.owner,
        "DELETE",
        &format!(
            "/v1/workspaces/{}/teams/{team}/members/{}",
            env.ws, env.owner.id
        ),
        None,
    )
    .await;
    let body = json!({"name": "crew-only", "visibility": "PRIVATE", "projectId": env.project, "teamId": team});
    let path = format!("/v1/workspaces/{}/channels", env.ws);
    // The owner is the Project's admin but not in the Team.
    let refused = env
        .call(&env.owner, "POST", &path, Some(body.clone()))
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.body);
    let made: i64 =
        sqlx::query_scalar("SELECT count(*) FROM penfii.channels WHERE name = 'crew-only'")
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert_eq!(made, 0);
    // A Project admin who is in the Team can; and a Project channel (no Team) is unaffected.
    env.grant(&pat, "ADMIN").await;
    let made = env.call(&pat, "POST", &path, Some(body)).await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
    let own = env
        .channel(&env.owner, "PRIVATE", &env.project, "no-team")
        .await;
    assert!(env.row(&env.owner, &own).await.is_some());
}

/// Losing the participation is never refused for the channels it leaves without a manager: they are
/// offered for recovery like any private channel without one.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn the_last_manager_losing_the_project_leaves_a_channel_to_recover(pool: PgPool) {
    let (env, pat) = setup(pool).await;
    env.grant(&pat, "ADMIN").await;
    let own = env.channel(&pat, "PRIVATE", &env.project, "pat-runs").await;
    env.ungrant(&pat).await; // not refused: the owner stays the Project's admin
    assert_eq!(env.row(&pat, &own).await, None);
    assert_eq!(env.without_manager().await, vec![own.clone()]);
    let counted: i32 = sqlx::query_scalar(
        "SELECT jsonb_array_length(data->'channelsWithoutManager') FROM penfii.audit_entries
         WHERE action_code = 'project.grant_removed'",
    )
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(counted, 1);
    // Recovery names the owner; naming pat (no longer in the Project) is refused.
    let refused = env
        .call(
            &env.owner,
            "POST",
            &format!("/v1/channels/{own}/manager-recovery"),
            Some(json!({"userId": pat.id})),
        )
        .await;
    assert_eq!(
        refused.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        refused.body
    );
    assert_eq!(refused.body["code"], "not_in_project");
    env.ok(
        &env.owner,
        "POST",
        &format!("/v1/channels/{own}/manager-recovery"),
        Some(json!({"userId": env.owner.id})),
    )
    .await;
    assert!(env.without_manager().await.is_empty());
}

/// Rows left by older versions go once, and nothing comes back with the participation.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn rows_left_behind_are_cleaned_once(pool: PgPool) {
    let (env, pat) = setup(pool).await;
    env.ungrant(&pat).await;
    env.plant(&pat, &env.private, "MANAGER").await;
    env.plant(&pat, &env.public, "MEMBER").await;
    // The rows of someone who still takes part stay: the owner's.
    let owner_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM penfii.channel_members WHERE user_id = $1")
            .bind(env.owner.id)
            .fetch_one(&env.pool)
            .await
            .unwrap();
    sqlx::raw_sql(include_str!(
        "../../../database/migrations/0006_project_channel_membership_cleanup.sql"
    ))
    .execute(&env.pool)
    .await
    .unwrap();
    assert_eq!(env.row(&pat, &env.private).await, None);
    assert_eq!(env.row(&pat, &env.public).await, None);
    let after: i64 =
        sqlx::query_scalar("SELECT count(*) FROM penfii.channel_members WHERE user_id = $1")
            .bind(env.owner.id)
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert_eq!(after, owner_rows);
    env.grant(&pat, "EDITOR").await;
    assert_eq!(env.pair(&pat).await, [SHUT, OPEN_WITHOUT_ROW]);
}

/// Channels of no Project are not touched by any of it.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn channels_of_no_project_keep_their_members(pool: PgPool) {
    let (env, pat) = setup(pool).await;
    let global = env
        .call(
            &env.owner,
            "POST",
            &format!("/v1/workspaces/{}/channels", env.ws),
            Some(json!({"name": "everyone", "visibility": "PRIVATE", "projectId": null})),
        )
        .await;
    assert_eq!(global.status, StatusCode::CREATED, "{}", global.body);
    let global = global.body["id"].as_str().unwrap().to_string();
    env.ok(
        &env.owner,
        "PUT",
        &format!("/v1/channels/{global}/members/{}", pat.id),
        None,
    )
    .await;
    env.ungrant(&pat).await;
    assert_eq!(env.standing(&pat, &global).await, FULL);
}

/// A participation change racing a channel member being added leaves no row behind for someone who
/// does not take part, whichever wins.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn losing_the_project_while_being_added_leaves_no_row(pool: PgPool) {
    let (env, pat) = setup(pool).await;
    let channel = env.private.clone();
    sqlx::query("DELETE FROM penfii.channel_members WHERE channel_id = $1 AND user_id = $2")
        .bind(channel.parse::<Uuid>().unwrap())
        .bind(pat.id)
        .execute(&env.pool)
        .await
        .unwrap();
    let add_path = format!("/v1/channels/{channel}/members/{}", pat.id);
    let remove_path = format!("/v1/projects/{}/grants/{}", env.project, pat.id);
    let add = env.call(&env.owner, "PUT", &add_path, None);
    let remove = env.call(&env.owner, "DELETE", &remove_path, None);
    let (added, removed) = tokio::join!(add, remove);
    assert_eq!(removed.status, StatusCode::NO_CONTENT, "{}", removed.body);
    assert!(
        added.status == StatusCode::OK || added.status.is_client_error(),
        "{} {}",
        added.status,
        added.body
    );
    assert_eq!(
        env.row(&pat, &channel).await,
        None,
        "no row for someone outside the Project"
    );
}
