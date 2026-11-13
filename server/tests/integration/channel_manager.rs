//! Private channels that lost their managers (FEA-051) and the warning before it happens (FEA-052).
//!
//! A private channel with no active manager (a `MANAGER` row whose person is an ACTIVE member of the
//! Workspace with an account that is not disabled; the meaning of "active" of the Project admins,
//! FEA-054) is listed for the Workspace owner and admin, whatever the cause, and one of them names a
//! manager. A public channel without a manager is an ordinary state and is not listed.

use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::common::{self, Person};

struct Env(common::Env);

impl std::ops::Deref for Env {
    type Target = common::Env;

    fn deref(&self) -> &common::Env {
        &self.0
    }
}

impl Env {
    async fn new(pool: PgPool) -> Env {
        Env(common::Env::new(pool).await)
    }

    async fn channel(
        &self,
        by: &Person,
        visibility: &str,
        project: Option<&str>,
        name: &str,
    ) -> Uuid {
        let created = self
            .call(
                by,
                "POST",
                &format!("/v1/workspaces/{}/channels", self.ws),
                Some(json!({"name": name, "visibility": visibility, "projectId": project})),
            )
            .await;
        assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
        created.body["id"].as_str().unwrap().parse().unwrap()
    }

    async fn put_in(&self, by: &Person, channel: Uuid, who: &Person) {
        let reply = self
            .call(
                by,
                "PUT",
                &format!("/v1/channels/{channel}/members/{}", who.id),
                None,
            )
            .await;
        assert!(reply.status.is_success(), "{}", reply.body);
    }

    async fn without_manager(&self, by: &Person) -> Vec<Value> {
        let reply = self
            .call(
                by,
                "GET",
                &format!("/v1/workspaces/{}/channels-without-manager", self.ws),
                None,
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
        reply.body["items"].as_array().unwrap().clone()
    }

    async fn listed(&self, channel: Uuid) -> bool {
        self.without_manager(&self.owner)
            .await
            .iter()
            .any(|c| c["id"] == json!(channel))
    }

    async fn recover(&self, by: &Person, channel: Uuid, who: Uuid) -> common::Reply {
        self.call(
            by,
            "POST",
            &format!("/v1/channels/{channel}/manager-recovery"),
            Some(json!({"userId": who})),
        )
        .await
    }

    async fn can_read(&self, who: &Person, channel: Uuid) -> bool {
        self.call(
            who,
            "GET",
            &format!("/v1/conversations/{channel}/messages"),
            None,
        )
        .await
        .status
            == StatusCode::OK
    }

    async fn role(&self, channel: Uuid, who: &Person) -> Option<String> {
        sqlx::query_scalar(
            "SELECT role FROM penfii.channel_members WHERE channel_id = $1 AND user_id = $2",
        )
        .bind(channel)
        .bind(who.id)
        .fetch_optional(&self.pool)
        .await
        .unwrap()
    }

    async fn remove(&self, who: &Person) {
        penfii_core::admin::remove_member(&self.pool, self.owner.id, self.ws, who.id)
            .await
            .unwrap();
    }

    async fn disable(&self, who: &Person) {
        penfii_core::admin::operator_set_account_disabled(&self.pool, &who.email, true)
            .await
            .unwrap();
    }

    async fn enable(&self, who: &Person) {
        penfii_core::admin::operator_set_account_disabled(&self.pool, &who.email, false)
            .await
            .unwrap();
    }

    async fn audit(&self, action: &str, target: Uuid) -> Vec<Value> {
        sqlx::query_scalar("SELECT data FROM penfii.audit_entries WHERE action_code = $1 AND target_id = $2 ORDER BY occurred_at")
            .bind(action)
            .bind(target)
            .fetch_all(&self.pool)
            .await
            .unwrap()
    }
}

/// Whatever the cause: no manager row at all, only a removed or a disabled manager, or a public
/// channel made private. A channel with an active manager and a public channel are not listed.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_private_channel_without_an_active_manager_is_listed_whatever_the_cause(pool: PgPool) {
    let env = Env::new(pool.clone()).await;
    let mia = env.person("mia", "MEMBER").await;
    let removed = env.person("removed", "ADMIN").await;
    let disabled = env.person("disabled", "ADMIN").await;
    let nobody = env.person("nobody", "ADMIN").await;
    let public = env.person("publicmaker", "ADMIN").await;
    let kept = env.person("kept", "ADMIN").await;

    let by_removal = env.channel(&removed, "PRIVATE", None, "by-removal").await;
    let by_disable = env.channel(&disabled, "PRIVATE", None, "by-disable").await;
    let by_no_row = env.channel(&nobody, "PRIVATE", None, "by-no-row").await;
    let run = env.channel(&kept, "PRIVATE", None, "has-a-manager").await;
    let was_public = env.channel(&public, "PUBLIC", None, "was-public").await;
    for (maker, channel) in [
        (&removed, by_removal),
        (&disabled, by_disable),
        (&nobody, by_no_row),
        (&kept, run),
    ] {
        env.put_in(maker, channel, &mia).await;
    }
    assert!(env.without_manager(&env.owner).await.is_empty());

    env.remove(&removed).await;
    env.disable(&disabled).await;
    sqlx::query("DELETE FROM penfii.channel_members WHERE channel_id = $1 AND role = 'MANAGER'")
        .bind(by_no_row)
        .execute(&pool)
        .await
        .unwrap();
    // The public channel's maker leaves: a public channel can do without a manager and is not listed.
    env.remove(&public).await;
    assert!(!env.listed(was_public).await);

    let items = env.without_manager(&env.owner).await;
    let ids: Vec<Value> = items.iter().map(|c| c["id"].clone()).collect();
    for channel in [by_removal, by_disable, by_no_row] {
        assert!(ids.contains(&json!(channel)), "{channel} is listed");
    }
    assert!(
        !ids.contains(&json!(run)),
        "a channel with an active manager is not listed"
    );
    assert_eq!(items.len(), 3);
    // Only names and counts: no content. The disabled manager is named as a former manager, the
    // removed one's row is gone with the removal.
    let by_disable_item = items.iter().find(|c| c["id"] == json!(by_disable)).unwrap();
    assert_eq!(
        by_disable_item["formerManagers"][0]["userId"],
        json!(disabled.id)
    );
    assert_eq!(by_disable_item["name"], "by-disable");
    assert!(by_disable_item.get("messages").is_none());
    let by_removal_item = items.iter().find(|c| c["id"] == json!(by_removal)).unwrap();
    assert_eq!(by_removal_item["formerManagers"], json!([]));
    assert_eq!(
        by_removal_item["memberCount"], 1,
        "mia, who can still read it"
    );

    // Made private, the public channel without a manager joins the list.
    let made_private = env
        .call(
            &env.owner,
            "PATCH",
            &format!("/v1/channels/{was_public}"),
            Some(json!({"visibility": "PRIVATE"})),
        )
        .await;
    assert_eq!(made_private.status, StatusCode::OK, "{}", made_private.body);
    assert!(env.listed(was_public).await);
    // Workspace owners and admins only.
    let denied = env
        .call(
            &mia,
            "GET",
            &format!("/v1/workspaces/{}/channels-without-manager", env.ws),
            None,
        )
        .await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN);
}

/// A Workspace owner or admin names a manager; nobody else may, a channel with a manager or a public
/// channel is refused, and the named person must be able to be one.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn only_a_workspace_owner_or_admin_recovers_a_channel_and_the_target_must_be_active(
    pool: PgPool,
) {
    let env = Env::new(pool.clone()).await;
    let boss = env.person("boss", "ADMIN").await;
    let maker = env.person("maker", "ADMIN").await;
    let mia = env.person("mia", "MEMBER").await;
    let gone = env.person("gone", "MEMBER").await;
    let paused = env.person("paused", "MEMBER").await;
    let outside = env.person("outside", "MEMBER").await;
    let channel = env.channel(&maker, "PRIVATE", None, "leads").await;
    for who in [&mia, &gone, &paused] {
        env.put_in(&maker, channel, who).await;
    }
    // It has a manager: refused (409) for everyone, and so is a public channel.
    let refused = env.recover(&env.owner, channel, mia.id).await;
    assert_eq!(refused.status, StatusCode::CONFLICT);
    assert_eq!(refused.body["code"], "channel_has_manager");
    let public = env.channel(&maker, "PUBLIC", None, "town").await;
    let not_private = env.recover(&env.owner, public, mia.id).await;
    assert_eq!(not_private.status, StatusCode::CONFLICT);
    assert_eq!(not_private.body["code"], "channel_not_private");
    let unknown = env.recover(&env.owner, Uuid::new_v4(), mia.id).await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);

    env.remove(&maker).await;
    env.remove(&gone).await;
    env.disable(&paused).await;
    assert!(env.listed(channel).await);

    // Not by a plain member, not by the channel's own member.
    for who in [&mia, &outside] {
        let reply = env.recover(who, channel, mia.id).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{}", reply.body);
    }
    // The person must be an active member of the Workspace whose account is not disabled.
    for who in [&gone, &paused] {
        let reply = env.recover(&boss, channel, who.id).await;
        assert!(
            reply.status.is_client_error() && reply.status != StatusCode::FORBIDDEN,
            "{}: {} {}",
            who.email,
            reply.status,
            reply.body
        );
        assert!(env.listed(channel).await, "refused, still listed");
    }
    let nobody = env.recover(&boss, channel, Uuid::new_v4()).await;
    assert!(nobody.status.is_client_error());
    assert!(env.listed(channel).await);
    assert!(
        env.audit("channel.manager_recovered", channel)
            .await
            .is_empty()
    );

    // A Workspace admin names a person who is already in the channel: they keep their place and
    // become its manager, and gain no new access.
    let done = env.recover(&boss, channel, mia.id).await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.body);
    assert_eq!(done.body["newAccess"], false);
    assert_eq!(env.role(channel, &mia).await.as_deref(), Some("MANAGER"));
    assert!(!env.listed(channel).await);
    let entries = env.audit("channel.manager_recovered", channel).await;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["userId"], json!(mia.id));
    assert_eq!(entries[0]["self"], false);
    assert_eq!(entries[0]["newAccess"], false);
    assert_eq!(entries[0]["previousRole"], "MEMBER");
    // The administrators who named nobody for themselves still cannot read it.
    assert!(!env.can_read(&boss, channel).await);
    assert!(!env.can_read(&env.owner, channel).await);
    // Recovered, so a second recovery is refused.
    let again = env.recover(&env.owner, channel, outside.id).await;
    assert_eq!(again.status, StatusCode::CONFLICT);
    assert_eq!(again.body["code"], "channel_has_manager");
}

/// Naming oneself is allowed: it opens the channel to the administrator, says so in the answer and
/// the audit entry. Naming someone who was not in it opens the channel to them too.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn naming_oneself_or_an_outsider_opens_the_channel_and_is_audited_as_new_access(
    pool: PgPool,
) {
    let env = Env::new(pool.clone()).await;
    let maker = env.person("maker", "ADMIN").await;
    let outsider = env.person("outsider", "MEMBER").await;
    let first = env.channel(&maker, "PRIVATE", None, "first").await;
    let second = env.channel(&maker, "PRIVATE", None, "second").await;
    env.remove(&maker).await;
    assert!(!env.can_read(&env.owner, first).await);

    let own = env.recover(&env.owner, first, env.owner.id).await;
    assert_eq!(own.status, StatusCode::OK, "{}", own.body);
    assert_eq!(own.body["newAccess"], true);
    assert!(env.can_read(&env.owner, first).await);
    let entry = &env.audit("channel.manager_recovered", first).await[0];
    assert_eq!(
        (entry["self"].clone(), entry["newAccess"].clone()),
        (json!(true), json!(true))
    );
    assert!(entry["previousRole"].is_null());

    let other = env.recover(&env.owner, second, outsider.id).await;
    assert_eq!(other.status, StatusCode::OK, "{}", other.body);
    assert_eq!(other.body["newAccess"], true);
    assert!(env.can_read(&outsider, second).await);
    assert!(
        !env.can_read(&env.owner, second).await,
        "the one who named them gains nothing"
    );
    let entry = &env.audit("channel.manager_recovered", second).await[0];
    assert_eq!(
        (entry["self"].clone(), entry["newAccess"].clone()),
        (json!(false), json!(true))
    );
}

/// A Project's channel takes only people of the Project, as when anyone is put in.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_project_channel_is_recovered_only_with_a_person_of_the_project(pool: PgPool) {
    let env = Env::new(pool.clone()).await;
    let lead = env.person("lead", "MEMBER").await;
    let insider = env.person("insider", "MEMBER").await;
    let outsider = env.person("outsider", "MEMBER").await;
    let project = env
        .call(
            &env.owner,
            "POST",
            &format!("/v1/workspaces/{}/projects", env.ws),
            Some(json!({"name": "Alpha", "teamIds": []})),
        )
        .await
        .body["id"]
        .as_str()
        .unwrap()
        .to_string();
    for (who, role) in [(&lead, "ADMIN"), (&insider, "EDITOR")] {
        let reply = env
            .call(
                &env.owner,
                "PUT",
                &format!("/v1/projects/{project}/grants/{}", who.id),
                Some(json!({"role": role})),
            )
            .await;
        assert!(reply.status.is_success(), "{}", reply.body);
    }
    let channel = env
        .channel(&lead, "PRIVATE", Some(&project), "alpha-leads")
        .await;
    env.remove(&lead).await;
    let items = env.without_manager(&env.owner).await;
    assert_eq!(items[0]["projectName"], "Alpha");
    let refused = env.recover(&env.owner, channel, outsider.id).await;
    assert!(
        refused.status.is_client_error() && refused.status != StatusCode::OK,
        "{}",
        refused.body
    );
    assert!(env.listed(channel).await);
    let done = env.recover(&env.owner, channel, insider.id).await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.body);
    assert!(env.can_read(&insider, channel).await);
}

/// A disabled manager does not count. Enabled again, they are a manager next to the one named in
/// the meantime; nobody is demoted and nothing is cleaned up.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn an_enabled_manager_and_the_recovered_one_are_managers_together(pool: PgPool) {
    let env = Env::new(pool.clone()).await;
    let old = env.person("old", "ADMIN").await;
    let mia = env.person("mia", "MEMBER").await;
    let channel = env.channel(&old, "PRIVATE", None, "pair").await;
    env.put_in(&old, channel, &mia).await;
    env.disable(&old).await;
    assert!(env.listed(channel).await);
    let done = env.recover(&env.owner, channel, mia.id).await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.body);
    assert!(!env.listed(channel).await);
    env.enable(&old).await;
    assert_eq!(env.role(channel, &old).await.as_deref(), Some("MANAGER"));
    assert_eq!(env.role(channel, &mia).await.as_deref(), Some("MANAGER"));
    assert!(!env.listed(channel).await);
    // Two managers: either may step aside, the last one may not.
    let old = Person {
        cookie: common::login(&env.app, &old.email, common::MEMBER_PASSWORD)
            .await
            .1
            .unwrap(),
        ..old
    };
    let step = env
        .call(
            &old,
            "DELETE",
            &format!("/v1/channels/{channel}/members/{}", old.id),
            None,
        )
        .await;
    assert!(step.status.is_success(), "{}", step.body);
    let last = env
        .call(
            &mia,
            "DELETE",
            &format!("/v1/channels/{channel}/members/{}", mia.id),
            None,
        )
        .await;
    assert_eq!(last.status, StatusCode::CONFLICT);
    assert_eq!(last.body["code"], "last_manager");
}

/// The warning before removing a person or disabling an account names the private channels that
/// would be left without an active manager, next to the Projects; nothing is refused for it.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn the_offboarding_preview_names_the_private_channels_left_without_a_manager(pool: PgPool) {
    let env = Env::new(pool.clone()).await;
    let maker = env.person("maker", "ADMIN").await;
    let co = env.person("co", "ADMIN").await;
    let sole = env.channel(&maker, "PRIVATE", None, "sole").await;
    let shared = env.channel(&maker, "PRIVATE", None, "shared").await;
    // A public channel the maker also manages: it is never named, a public channel can do without one.
    env.channel(&maker, "PUBLIC", None, "open").await;
    env.put_in(&maker, shared, &co).await;
    let promote = env
        .call(
            &maker,
            "PATCH",
            &format!("/v1/channels/{shared}/members/{}", co.id),
            Some(json!({"role": "MANAGER"})),
        )
        .await;
    assert!(promote.status.is_success(), "{}", promote.body);

    let preview = |who: &Person| {
        let (env, path) = (
            &env,
            format!(
                "/v1/workspaces/{}/members/{}/offboarding-impact",
                env.ws, who.id
            ),
        );
        async move {
            let reply = env.call(&env.owner, "GET", &path, None).await;
            assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
            reply.body
        }
    };
    let body = preview(&maker).await;
    let channels = body["channels"].as_array().unwrap();
    assert_eq!(channels.len(), 1, "{body}");
    assert_eq!(channels[0]["channelId"], json!(sole));
    assert_eq!(channels[0]["name"], "sole");
    assert!(channels[0]["projectId"].is_null());
    assert!(body["projects"].as_array().unwrap().is_empty());
    // With the co-manager gone too, the shared channel has none left either.
    assert!(
        preview(&co).await["channels"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    // A disabled co-manager does not keep the channel: the preview for the other one names it.
    env.disable(&co).await;
    let names: Vec<String> = preview(&maker).await["channels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(names, vec!["shared".to_string(), "sole".to_string()]);

    // The operator's disable shows the same, and the removal is not refused for it.
    let change = penfii_core::admin::operator_set_account_disabled(&pool, &maker.email, true)
        .await
        .unwrap();
    let impact = &change.impacts[0];
    assert_eq!(
        impact
            .channels
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        vec!["shared", "sole"]
    );
    penfii_core::admin::operator_set_account_disabled(&pool, &maker.email, false)
        .await
        .unwrap();
    env.remove(&maker).await;
    let removed: Value = sqlx::query_scalar(
        "SELECT data FROM penfii.audit_entries WHERE action_code = 'member.removed' AND target_id = $1",
    )
    .bind(maker.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let mut named: Vec<String> = removed["channelsWithoutManager"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_str().unwrap().to_string())
        .collect();
    named.sort();
    let mut expect = vec![sole.to_string(), shared.to_string()];
    expect.sort();
    assert_eq!(named, expect);
    assert_eq!(
        removed["clearedChannels"],
        json!({"private": 2, "public": 1})
    );
    assert!(env.listed(sole).await && env.listed(shared).await);
}

/// Two recoveries at once name one manager: the channel is locked, the second finds one.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn recoveries_at_once_name_one_manager(pool: PgPool) {
    let env = Env::new(pool.clone()).await;
    let maker = env.person("maker", "ADMIN").await;
    let boss = env.person("boss", "ADMIN").await;
    let one = env.person("one", "MEMBER").await;
    let two = env.person("two", "MEMBER").await;
    let channel = env.channel(&maker, "PRIVATE", None, "race").await;
    env.put_in(&maker, channel, &one).await;
    env.put_in(&maker, channel, &two).await;
    env.remove(&maker).await;
    for _ in 0..4 {
        // Reset: nobody manages it.
        sqlx::query("UPDATE penfii.channel_members SET role = 'MEMBER' WHERE channel_id = $1")
            .bind(channel)
            .execute(&pool)
            .await
            .unwrap();
        let (a, b) = tokio::join!(
            env.recover(&env.owner, channel, one.id),
            env.recover(&boss, channel, two.id)
        );
        let mut statuses = [a.status, b.status];
        statuses.sort();
        assert_eq!(
            statuses,
            [StatusCode::OK, StatusCode::CONFLICT],
            "{} / {}",
            a.body,
            b.body
        );
        let managers: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM penfii.channel_members WHERE channel_id = $1 AND role = 'MANAGER'",
        )
        .bind(channel)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(managers, 1);
    }
}

/// The Workspace removal of a channel's only manager racing the recovery of the same channel: either
/// order ends in a consistent state, never a deadlock or a server error. Recovery first is refused
/// because the manager is still active and the removal then leaves the channel to recover; removal
/// first leaves the recovery free to succeed.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn removing_the_only_manager_while_recovering_ends_consistent(pool: PgPool) {
    let env = Env::new(pool.clone()).await;
    for round in 0..6 {
        let maker = env.person(&format!("maker{round}"), "ADMIN").await;
        let channel = env
            .channel(&maker, "PRIVATE", None, &format!("held-{round}"))
            .await;
        let (removed, recovered) = tokio::join!(
            penfii_core::admin::remove_member(&pool, env.owner.id, env.ws, maker.id),
            penfii_core::conversation::recovery::recover_manager(
                &pool,
                env.owner.id,
                channel,
                env.owner.id
            )
        );
        removed.unwrap_or_else(|error| panic!("round {round}: removal failed: {error:?}"));
        match recovered {
            Ok(_) => {
                assert_eq!(
                    env.role(channel, &env.owner).await.as_deref(),
                    Some("MANAGER"),
                    "round {round}: the named person is the manager"
                );
                assert!(!env.listed(channel).await, "round {round}");
            }
            Err(error) => {
                let text = format!("{error:?}");
                assert!(
                    text.contains("channel_has_manager"),
                    "round {round}: only the guard may refuse: {text}"
                );
                assert_eq!(env.role(channel, &maker).await, None, "round {round}");
                assert!(
                    env.listed(channel).await,
                    "round {round}: left without a manager, so recoverable"
                );
                env.recover(&env.owner, channel, env.owner.id).await;
            }
        }
        assert_eq!(
            env.role(channel, &maker).await,
            None,
            "round {round}: the removed manager's row is gone"
        );
    }
}

/// A recovery racing another person being made manager of the same Project channel: at least one
/// manager remains and nothing fails with a server error, whichever order wins.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn recovering_while_someone_is_made_manager_keeps_a_manager(pool: PgPool) {
    let env = Env::new(pool.clone()).await;
    let lead = env.person("lead", "MEMBER").await;
    let pat = env.person("pat", "MEMBER").await;
    let created = env
        .call(
            &env.owner,
            "POST",
            &format!("/v1/workspaces/{}/projects", env.ws),
            Some(json!({"name": "Alpha", "teamIds": []})),
        )
        .await;
    let project = created.body["id"].as_str().unwrap().to_string();
    for (who, role) in [(&lead, "ADMIN"), (&pat, "EDITOR")] {
        let reply = env
            .call(
                &env.owner,
                "PUT",
                &format!("/v1/projects/{project}/grants/{}", who.id),
                Some(json!({"role": role})),
            )
            .await;
        assert!(reply.status.is_success(), "{}", reply.body);
    }
    for round in 0..4 {
        let channel = env
            .channel(&lead, "PRIVATE", Some(&project), &format!("run-{round}"))
            .await;
        env.put_in(&lead, channel, &pat).await;
        // The channel's only manager leaves the Workspace's reach: the lead's row is taken away.
        sqlx::query("DELETE FROM penfii.channel_members WHERE channel_id = $1 AND user_id = $2")
            .bind(channel)
            .bind(lead.id)
            .execute(&pool)
            .await
            .unwrap();
        assert!(env.listed(channel).await);
        let set_path = format!("/v1/channels/{channel}/members/{}", pat.id);
        let make = env.call(&lead, "PATCH", &set_path, Some(json!({"role": "MANAGER"})));
        let recover = env.recover(&env.owner, channel, env.owner.id);
        let (made, recovered) = tokio::join!(make, recover);
        assert!(
            !made.status.is_server_error() && !recovered.status.is_server_error(),
            "round {round}: {} {} / {} {}",
            made.status,
            made.body,
            recovered.status,
            recovered.body
        );
        let managers: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM penfii.channel_members WHERE channel_id = $1 AND role = 'MANAGER'",
        )
        .bind(channel)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(managers >= 1, "round {round}: a manager remains");
        assert!(!env.listed(channel).await, "round {round}");
    }
}
