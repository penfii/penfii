//! Running a private channel without reading it (FEA-049) and changing its visibility (FEA-050).
//!
//! The admin of a Project runs the people and the settings of its private channels, but that is not a
//! way into them: an admin who is not in the channel reads nothing, posts nothing, cannot put themself
//! in, and cannot open the channel by turning it public. The channel's manager can do all of it.

use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::common::{self, Person, doc};

struct Env {
    env: common::Env,
    project: String,
    /// The Project's private channel, made by the owner (its manager).
    private: String,
    /// Admin of the Project, not in the channel.
    away: Person,
    /// Admin of the Project, put in the channel as a plain member.
    inside: Person,
    /// Plain member of the channel.
    member: Person,
    /// Editor of the Project, not in the channel.
    editor: Person,
    /// Workspace admin who does not take part in the Project.
    wsadmin: Person,
}

impl std::ops::Deref for Env {
    type Target = common::Env;

    fn deref(&self) -> &common::Env {
        &self.env
    }
}

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

    async fn status(
        &self,
        who: &Person,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> StatusCode {
        self.call(who, method, path, body).await.status
    }

    fn members_path(&self, channel: &str) -> String {
        format!("/v1/channels/{channel}/members")
    }

    async fn rows(&self, channel: &str) -> Vec<(Uuid, String)> {
        sqlx::query_as(
            "SELECT user_id, role FROM penfii.channel_members WHERE channel_id = $1 ORDER BY user_id",
        )
        .bind(channel.parse::<Uuid>().unwrap())
        .fetch_all(&self.pool)
        .await
        .unwrap()
    }

    async fn can_read(&self, who: &Person, channel: &str) -> bool {
        self.status(
            who,
            "GET",
            &format!("/v1/conversations/{channel}/messages"),
            None,
        )
        .await
            == StatusCode::OK
    }

    async fn audits(&self, action: &str) -> Vec<Value> {
        let rows: Vec<(Value,)> = sqlx::query_as(
            "SELECT data FROM penfii.audit_entries WHERE action_code = $1 ORDER BY occurred_at",
        )
        .bind(action)
        .fetch_all(&self.pool)
        .await
        .unwrap();
        rows.into_iter().map(|(data,)| data).collect()
    }
}

async fn setup(pool: PgPool) -> Env {
    let env = common::Env::new(pool).await;
    let away = env.person("away", "MEMBER").await;
    let inside = env.person("inside", "MEMBER").await;
    let member = env.person("member", "MEMBER").await;
    let editor = env.person("editor", "MEMBER").await;
    let wsadmin = env.person("wsadmin", "ADMIN").await;
    let created = env
        .call(
            &env.owner,
            "POST",
            &format!("/v1/workspaces/{}/projects", env.ws),
            Some(json!({"name": "Alpha", "teamIds": []})),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    let project = created.body["id"].as_str().unwrap().to_string();
    let mut env = Env {
        env,
        project: project.clone(),
        private: String::new(),
        away,
        inside,
        member,
        editor,
        wsadmin,
    };
    for (who, role) in [
        (&env.away, "ADMIN"),
        (&env.inside, "ADMIN"),
        (&env.member, "EDITOR"),
        (&env.editor, "EDITOR"),
    ] {
        env.ok(
            &env.owner,
            "PUT",
            &format!("/v1/projects/{project}/grants/{}", who.id),
            Some(json!({"role": role})),
        )
        .await;
    }
    let channel = env
        .ok(
            &env.owner,
            "POST",
            &format!("/v1/workspaces/{}/channels", env.ws),
            Some(json!({"name": "leads", "visibility": "PRIVATE", "projectId": project})),
        )
        .await;
    env.private = channel["id"].as_str().unwrap().to_string();
    for who in [&env.inside, &env.member] {
        env.ok(
            &env.owner,
            "PUT",
            &format!("/v1/channels/{}/members/{}", env.private, who.id),
            None,
        )
        .await;
    }
    env
}

fn webhook_body(name: &str) -> Value {
    json!({"name": name, "authMode": "URL_SECRET", "displayName": name})
}

/// What an admin who is not in the private channel may and may not do.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn an_admin_outside_the_channel_runs_its_people_but_cannot_read_or_join_it(pool: PgPool) {
    let env = setup(pool).await;
    let away = &env.away;
    let channel = &env.private;

    // Nothing of the content: reading, posting, the Space list.
    assert!(!env.can_read(away, channel).await);
    let posted = env
        .status(
            away,
            "POST",
            &format!("/v1/conversations/{channel}/messages"),
            Some(json!({"body": doc("hi"), "bodySchemaVersion": 1, "threadRootId": null, "replyToId": null})),
        )
        .await;
    assert_eq!(posted, StatusCode::NOT_FOUND);
    let space = env
        .ok(
            away,
            "GET",
            &format!("/v1/workspaces/{}/space", env.ws),
            None,
        )
        .await;
    assert!(!space.to_string().contains(channel.as_str()));

    // The people: seen, put in, taken out, made manager, with the switch and the description.
    let listed = env.ok(away, "GET", &env.members_path(channel), None).await;
    assert!(
        listed.to_string().contains(&env.member.id.to_string()),
        "{listed}"
    );
    env.ok(
        away,
        "PUT",
        &format!("/v1/channels/{channel}/members/{}", env.editor.id),
        None,
    )
    .await;
    env.ok(
        away,
        "PATCH",
        &format!("/v1/channels/{channel}/members/{}", env.editor.id),
        Some(json!({"role": "MANAGER"})),
    )
    .await;
    env.ok(
        away,
        "PATCH",
        &format!("/v1/channels/{channel}"),
        Some(json!({"allowMemberInvites": true, "description": "run by the Project"})),
    )
    .await;
    let removed = env
        .call(
            away,
            "DELETE",
            &format!("/v1/channels/{channel}/members/{}", env.member.id),
            None,
        )
        .await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT, "{}", removed.body);
    assert!(!env.can_read(&env.member, channel).await);

    // Not themself: not as a member, not as a manager.
    let join_self = env
        .call(
            away,
            "PUT",
            &format!("/v1/channels/{channel}/members/{}", away.id),
            None,
        )
        .await;
    assert_eq!(
        join_self.status,
        StatusCode::FORBIDDEN,
        "{}",
        join_self.body
    );
    let manage_self = env
        .call(
            away,
            "PATCH",
            &format!("/v1/channels/{channel}/members/{}", away.id),
            Some(json!({"role": "MANAGER"})),
        )
        .await;
    assert_eq!(
        manage_self.status,
        StatusCode::FORBIDDEN,
        "{}",
        manage_self.body
    );
    assert!(
        env.rows(channel)
            .await
            .iter()
            .all(|(user, _)| *user != away.id)
    );

    // Every change by an admin who cannot read says so in the audit log.
    for action in [
        "channel.member_added",
        "channel.member_role_changed",
        "channel.member_removed",
        "channel.allow_member_invites_changed",
    ] {
        let entries = env.audits(action).await;
        assert!(
            entries
                .iter()
                .any(|data| data["actorCanRead"] == json!(false)),
            "{action}: {entries:?}"
        );
    }
}

/// An outsider, a plain member and a Workspace admin outside the Project run nothing.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn only_the_manager_and_the_projects_admin_run_the_channel(pool: PgPool) {
    let env = setup(pool).await;
    let channel = &env.private;
    let target = format!("/v1/channels/{channel}/members/{}", env.editor.id);
    // A plain member (invites are off): readable, so 403 rather than "not found".
    assert_eq!(
        env.status(&env.member, "PUT", &target, None).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        env.status(
            &env.member,
            "PATCH",
            &target,
            Some(json!({"role": "MANAGER"}))
        )
        .await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        env.status(
            &env.member,
            "PATCH",
            &format!("/v1/channels/{channel}"),
            Some(json!({"allowMemberInvites": true}))
        )
        .await,
        StatusCode::FORBIDDEN
    );
    // Not in the Project, or in it without being admin: the channel is not theirs to know.
    for who in [&env.editor, &env.wsadmin] {
        for (method, path, body) in [
            ("PUT", target.clone(), None),
            ("GET", env.members_path(channel), None),
            (
                "PATCH",
                format!("/v1/channels/{channel}"),
                Some(json!({"description": "x"})),
            ),
            ("GET", format!("/v1/channels/{channel}/webhooks"), None),
        ] {
            assert_eq!(
                env.status(who, method, &path, body).await,
                StatusCode::NOT_FOUND,
                "{method} {path}"
            );
        }
    }
    // The Project's channel list for admins is for admins.
    let listed = env
        .call(
            &env.member,
            "GET",
            &format!("/v1/projects/{}/private-channels", env.project),
            None,
        )
        .await;
    assert!(listed.status.is_client_error(), "{}", listed.body);
}

/// The admin finds the private channels of the Project without reading them.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn the_admin_lists_the_private_channels_with_names_and_counts_only(pool: PgPool) {
    let env = setup(pool).await;
    let posted = env
        .ok(
            &env.owner,
            "POST",
            &format!("/v1/conversations/{}/messages", env.private),
            Some(json!({"body": doc("a secret"), "bodySchemaVersion": 1, "threadRootId": null, "replyToId": null})),
        )
        .await;
    assert!(posted["id"].is_string());
    let listed = env
        .ok(
            &env.away,
            "GET",
            &format!("/v1/projects/{}/private-channels", env.project),
            None,
        )
        .await;
    let items = listed["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{listed}");
    let item = &items[0];
    assert_eq!(item["id"], json!(env.private));
    assert_eq!(item["name"], "leads");
    assert_eq!(item["memberCount"], 3);
    assert_eq!(item["managerCount"], 1);
    assert_eq!(item["isMember"], false);
    assert!(!listed.to_string().contains("a secret"));
    // The same list for an admin who is in the channel marks it.
    let mine = env
        .ok(
            &env.inside,
            "GET",
            &format!("/v1/projects/{}/private-channels", env.project),
            None,
        )
        .await;
    assert_eq!(mine["items"][0]["isMember"], true);
}

/// Webhooks of a private channel: the posting credential is for someone who may post; seeing the list
/// and revoking only take running the channel.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn webhooks_of_a_private_channel_follow_who_may_post_and_who_runs_it(pool: PgPool) {
    let env = setup(pool).await;
    let channel = &env.private;
    let hooks = format!("/v1/channels/{channel}/webhooks");
    // The manager issues one.
    let made = env
        .ok(&env.owner, "POST", &hooks, Some(webhook_body("deploys")))
        .await;
    let hook = made["webhook"]["id"]
        .as_str()
        .or_else(|| made["id"].as_str())
        .unwrap()
        .to_string();

    // The admin outside: sees the list and revokes, but issues and rotates nothing.
    let seen = env.ok(&env.away, "GET", &hooks, None).await;
    assert!(seen.to_string().contains(&hook), "{seen}");
    assert_eq!(
        env.status(&env.away, "POST", &hooks, Some(webhook_body("mine")))
            .await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        env.status(
            &env.away,
            "POST",
            &format!("/v1/webhooks/{hook}/rotate"),
            None
        )
        .await,
        StatusCode::FORBIDDEN
    );
    // The admin inside issues and rotates (they may post), and a plain member does nothing.
    env.ok(&env.inside, "POST", &hooks, Some(webhook_body("inside")))
        .await;
    env.ok(
        &env.inside,
        "POST",
        &format!("/v1/webhooks/{hook}/rotate"),
        None,
    )
    .await;
    for (method, path, body) in [
        ("GET", hooks.clone(), None),
        ("POST", hooks.clone(), Some(webhook_body("no"))),
        ("POST", format!("/v1/webhooks/{hook}/revoke"), None),
    ] {
        assert_eq!(
            env.status(&env.member, method, &path, body).await,
            StatusCode::FORBIDDEN,
            "{method} {path}"
        );
    }
    // Revoking by the admin who cannot read ends it and is recorded as such.
    let revoked = env
        .ok(
            &env.away,
            "POST",
            &format!("/v1/webhooks/{hook}/revoke"),
            None,
        )
        .await;
    assert_eq!(revoked["revoked"], true, "{revoked}");
    let entries = env.audits("webhook.revoked").await;
    assert!(
        entries
            .iter()
            .any(|data| data["actorCanRead"] == json!(false)),
        "{entries:?}"
    );
    // Nothing of this opened the channel.
    assert!(!env.can_read(&env.away, channel).await);
}

/// Making a private channel public is the manager's alone; making it private is for the people who run
/// it. Neither direction makes or removes a membership.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn visibility_changes_by_who_may_and_touch_no_membership(pool: PgPool) {
    let env = setup(pool).await;
    let channel = &env.private;
    let path = format!("/v1/channels/{channel}");
    let before = env.rows(channel).await;

    // Private -> public: the admin outside, the admin inside and the Workspace admin are refused.
    assert_eq!(
        env.status(
            &env.away,
            "PATCH",
            &path,
            Some(json!({"visibility": "PUBLIC"}))
        )
        .await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        env.status(
            &env.inside,
            "PATCH",
            &path,
            Some(json!({"visibility": "PUBLIC"}))
        )
        .await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        env.status(
            &env.wsadmin,
            "PATCH",
            &path,
            Some(json!({"visibility": "PUBLIC"}))
        )
        .await,
        StatusCode::NOT_FOUND
    );
    assert!(
        !env.can_read(&env.away, channel).await,
        "the refusal gave no way in"
    );
    assert!(!env.can_read(&env.editor, channel).await);
    assert_eq!(env.rows(channel).await, before);

    // The manager may. People of the Project who are not in the channel now read it, by the public
    // rule, with no row; the rows are as they were.
    env.ok(
        &env.owner,
        "PATCH",
        &path,
        Some(json!({"visibility": "PUBLIC"})),
    )
    .await;
    assert_eq!(env.rows(channel).await, before, "no membership made");
    assert!(env.can_read(&env.editor, channel).await);
    assert!(env.can_read(&env.away, channel).await);

    // Public -> private by the admin who is not in it: allowed, and they lose it themself. Those in it
    // keep it, those who read it only as the Project's people do not, and no row is made or removed.
    env.ok(
        &env.away,
        "PATCH",
        &path,
        Some(json!({"visibility": "PRIVATE"})),
    )
    .await;
    assert_eq!(env.rows(channel).await, before, "no membership removed");
    assert!(!env.can_read(&env.away, channel).await);
    assert!(!env.can_read(&env.editor, channel).await);
    assert!(env.can_read(&env.member, channel).await);
    assert!(env.can_read(&env.owner, channel).await);

    // Both are in the audit log, with whether the one who did it was in the channel.
    let entries = env.audits("channel.visibility_changed").await;
    assert_eq!(entries.len(), 2, "{entries:?}");
    assert_eq!(
        (entries[0]["from"].as_str(), entries[0]["to"].as_str()),
        (Some("PRIVATE"), Some("PUBLIC"))
    );
    assert_eq!(entries[0]["actorWasMember"], true);
    assert_eq!(
        (entries[1]["from"].as_str(), entries[1]["to"].as_str()),
        (Some("PUBLIC"), Some("PRIVATE"))
    );
    assert_eq!(entries[1]["actorWasMember"], false);
    assert!(entries[1]["peopleWithoutMembership"].as_i64().unwrap() >= 1);
}

/// The last manager is kept whoever takes people out, and a Project admin's removal of one of two
/// managers is allowed.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn taking_the_last_manager_out_is_refused(pool: PgPool) {
    let env = setup(pool).await;
    let channel = &env.private;
    let owner_path = format!("/v1/channels/{channel}/members/{}", env.owner.id);
    let refused = env.call(&env.away, "DELETE", &owner_path, None).await;
    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.body);
    assert_eq!(refused.body["code"], "last_manager");
    let demote = env
        .call(
            &env.away,
            "PATCH",
            &owner_path,
            Some(json!({"role": "MEMBER"})),
        )
        .await;
    assert_eq!(demote.status, StatusCode::CONFLICT, "{}", demote.body);
    // With a second manager the first can go.
    env.ok(
        &env.away,
        "PATCH",
        &format!("/v1/channels/{channel}/members/{}", env.inside.id),
        Some(json!({"role": "MANAGER"})),
    )
    .await;
    let removed = env.call(&env.away, "DELETE", &owner_path, None).await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT, "{}", removed.body);
}

/// The Workspace owner or admin is not a manager of a private channel; only the recovery of a channel
/// without one (FEA-051) gives them a hand in it.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_workspace_admin_does_not_run_a_private_channel_of_a_global_kind_either(pool: PgPool) {
    let env = setup(pool).await;
    let global = env
        .ok(
            &env.owner,
            "POST",
            &format!("/v1/workspaces/{}/channels", env.ws),
            Some(json!({"name": "board", "visibility": "PRIVATE"})),
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let admin_path = format!("/v1/channels/{global}/members/{}", env.member.id);
    assert_eq!(
        env.status(&env.wsadmin, "PUT", &admin_path, None).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        env.status(
            &env.wsadmin,
            "PATCH",
            &format!("/v1/channels/{global}"),
            Some(json!({"visibility": "PUBLIC"}))
        )
        .await,
        StatusCode::NOT_FOUND
    );
}
