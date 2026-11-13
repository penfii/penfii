//! Putting people into a channel: who may do it, and who may be put there. A private channel is
//! open to the people in `channel_members` only, so this is the way anyone but its creator gets in.

use axum::http::StatusCode;
use penfii_core::files::ObjectStore;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use crate::common::{self, Person, doc};

/// The shared fixture over an in-memory object store.
struct Env(common::Env);

impl std::ops::Deref for Env {
    type Target = common::Env;

    fn deref(&self) -> &common::Env {
        &self.0
    }
}

async fn setup(pool: PgPool) -> Env {
    Env(common::Env::with_objects(pool, ObjectStore::in_memory()).await)
}

impl Env {
    async fn channel(&self, who: &Person, visibility: &str, project: Option<&str>) -> String {
        let created = self
            .call(
                who,
                "POST",
                &format!("/v1/workspaces/{}/channels", self.ws),
                Some(json!({"name": "leads", "visibility": visibility, "projectId": project})),
            )
            .await;
        assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
        created.body["id"].as_str().unwrap().to_string()
    }

    fn members_path(channel: &str) -> String {
        format!("/v1/channels/{channel}/members")
    }

    async fn add(&self, who: &Person, channel: &str, person: &Person) -> common::Reply {
        self.call(
            who,
            "PUT",
            &format!("{}/{}", Self::members_path(channel), person.id),
            None,
        )
        .await
    }

    async fn can_read(&self, who: &Person, channel: &str) -> bool {
        let reply = self
            .call(
                who,
                "GET",
                &format!("/v1/conversations/{channel}/messages"),
                None,
            )
            .await;
        reply.status == StatusCode::OK
    }
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_manager_puts_people_into_a_private_channel(pool: PgPool) {
    let env = setup(pool).await;
    let bob = env.person("bob", "MEMBER").await;
    let carol = env.person("carol", "MEMBER").await;
    let channel = env.channel(&env.owner, "PRIVATE", None).await;

    // Nobody but the creator gets in on their own.
    assert!(!env.can_read(&bob, &channel).await);
    assert_eq!(
        env.call(&bob, "GET", &Env::members_path(&channel), None)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        env.add(&bob, &channel, &bob).await.status,
        StatusCode::NOT_FOUND,
        "a stranger cannot even see it"
    );
    let joined = env
        .call(&bob, "POST", &format!("/v1/channels/{channel}/join"), None)
        .await;
    assert_eq!(joined.status, StatusCode::NOT_FOUND);

    let added = env.add(&env.owner, &channel, &bob).await;
    assert_eq!(added.status, StatusCode::OK, "{}", added.body);
    assert_eq!(added.body["role"], "MEMBER");
    assert_eq!(added.body["displayName"], "bob");
    assert!(env.can_read(&bob, &channel).await);
    let posted = env
        .call(
            &bob,
            "POST",
            &format!("/v1/conversations/{channel}/messages"),
            Some(json!({"bodySchemaVersion": 1, "body": doc("hello")})),
        )
        .await;
    assert_eq!(posted.status, StatusCode::CREATED, "{}", posted.body);

    // Repeating it changes nothing.
    let again = env.add(&env.owner, &channel, &bob).await;
    assert_eq!(again.status, StatusCode::OK);
    assert_eq!(again.body["joinedAt"], added.body["joinedAt"]);

    // The list: the manager first, then the member; a member may look, a stranger may not.
    let list = env
        .call(&bob, "GET", &Env::members_path(&channel), None)
        .await;
    assert_eq!(list.status, StatusCode::OK, "{}", list.body);
    let roles: Vec<&str> = list.body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["role"].as_str().unwrap())
        .collect();
    assert_eq!(roles, ["MANAGER", "MEMBER"]);
    assert_eq!(
        env.call(&carol, "GET", &Env::members_path(&channel), None)
            .await
            .status,
        StatusCode::NOT_FOUND
    );

    // A plain member does not manage the channel, so cannot bring others in.
    let refused = env.add(&bob, &channel, &carol).await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.body);
    assert!(!env.can_read(&carol, &channel).await);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn only_people_of_the_workspace_can_be_put_into_a_channel(pool: PgPool) {
    let env = setup(pool).await;
    let bob = env.person("bob", "MEMBER").await;
    let channel = env.channel(&env.owner, "PRIVATE", None).await;

    let unknown = env
        .call(
            &env.owner,
            "PUT",
            &format!("{}/{}", Env::members_path(&channel), Uuid::new_v4()),
            None,
        )
        .await;
    assert_eq!(unknown.status, StatusCode::BAD_REQUEST, "{}", unknown.body);

    // A person who left the Workspace cannot be put in either.
    let removed = env
        .call(
            &env.owner,
            "DELETE",
            &format!("/v1/workspaces/{}/members/{}", env.ws, bob.id),
            None,
        )
        .await;
    assert!(
        removed.status.is_success() || removed.status == StatusCode::FORBIDDEN,
        "{}",
        removed.body
    );
    if removed.status.is_success() {
        assert_eq!(
            env.add(&env.owner, &channel, &bob).await.status,
            StatusCode::BAD_REQUEST
        );
    }
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_projects_channel_takes_only_people_of_the_project(pool: PgPool) {
    let env = setup(pool).await;
    let inside = env.person("inside", "MEMBER").await;
    let outside = env.person("outside", "MEMBER").await;
    let team = env
        .call(
            &env.owner,
            "POST",
            &format!("/v1/workspaces/{}/teams", env.ws),
            Some(json!({"name": "Dev", "members": [{"userId": inside.id, "role": "MANAGER"}]})),
        )
        .await;
    let team_id = team.body["id"].as_str().unwrap().to_string();
    let project = env
        .call(
            &env.owner,
            "POST",
            &format!("/v1/workspaces/{}/projects", env.ws),
            Some(json!({"name": "Alpha", "teamIds": [team_id]})),
        )
        .await;
    let project_id = project.body["id"].as_str().unwrap().to_string();
    let participation = env
        .call(
            &env.owner,
            "PUT",
            &format!("/v1/projects/{project_id}/teams/{team_id}/participation"),
            Some(json!({"role": "EDITOR"})),
        )
        .await;
    assert_eq!(
        participation.status,
        StatusCode::NO_CONTENT,
        "{}",
        participation.body
    );

    let channel = env.channel(&env.owner, "PRIVATE", Some(&project_id)).await;
    // The project's own people go in; the one who does not take part is refused, so a channel never
    // gives access that the Project does not.
    assert_eq!(
        env.add(&env.owner, &channel, &inside).await.status,
        StatusCode::OK
    );
    assert!(env.can_read(&inside, &channel).await);
    let refused = env.add(&env.owner, &channel, &outside).await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.body);
    assert!(!env.can_read(&outside, &channel).await);
    let _ = env.owner.id;
}

impl Env {
    async fn leave(&self, who: &Person, channel: &str, person: &Person) -> common::Reply {
        self.call(
            who,
            "DELETE",
            &format!("{}/{}", Self::members_path(channel), person.id),
            None,
        )
        .await
    }
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_member_leaves_a_private_channel_and_only_themselves(pool: PgPool) {
    let env = setup(pool).await;
    let bob = env.person("bob", "MEMBER").await;
    let carol = env.person("carol", "MEMBER").await;
    let channel = env.channel(&env.owner, "PRIVATE", None).await;
    assert_eq!(
        env.add(&env.owner, &channel, &bob).await.status,
        StatusCode::OK
    );
    assert_eq!(
        env.add(&env.owner, &channel, &carol).await.status,
        StatusCode::OK
    );

    // A plain member does not take another person out; the channel's manager does (FEA-049).
    let refused = env.leave(&bob, &channel, &carol).await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.body);
    assert!(env.can_read(&carol, &channel).await);
    let removed = env.leave(&env.owner, &channel, &carol).await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT, "{}", removed.body);
    assert!(!env.can_read(&carol, &channel).await);
    assert_eq!(
        env.add(&env.owner, &channel, &carol).await.status,
        StatusCode::OK
    );
    assert!(env.can_read(&carol, &channel).await);

    // A member leaves: the channel closes to them, the others stay.
    let left = env.leave(&bob, &channel, &bob).await;
    assert_eq!(left.status, StatusCode::NO_CONTENT, "{}", left.body);
    assert!(!env.can_read(&bob, &channel).await);
    assert!(env.can_read(&carol, &channel).await);
    // They are no longer in the channel, so it is not theirs to see or to leave again.
    assert_eq!(
        env.leave(&bob, &channel, &bob).await.status,
        StatusCode::NOT_FOUND
    );
    // They can be put back.
    assert_eq!(
        env.add(&env.owner, &channel, &bob).await.status,
        StatusCode::OK
    );
    assert!(env.can_read(&bob, &channel).await);

    // The domain events have both removals: the member's own leaving and the manager taking carol out.
    let events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.domain_events WHERE event_kind = 'channel.member_removed'",
    )
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(events, 2);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn the_last_manager_cannot_leave_and_a_public_channel_is_not_left_here(pool: PgPool) {
    let env = setup(pool).await;
    let bob = env.person("bob", "MEMBER").await;
    let channel = env.channel(&env.owner, "PRIVATE", None).await;
    assert_eq!(
        env.add(&env.owner, &channel, &bob).await.status,
        StatusCode::OK
    );

    let stuck = env.leave(&env.owner, &channel, &env.owner).await;
    assert_eq!(stuck.status, StatusCode::CONFLICT, "{}", stuck.body);
    assert_eq!(stuck.body["code"], "last_manager");
    assert!(env.can_read(&env.owner, &channel).await);

    let public = env.channel(&env.owner, "PUBLIC", None).await;
    let refused = env.leave(&env.owner, &public, &env.owner).await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.body);
    // A stranger of a private channel learns nothing from trying.
    assert_eq!(
        env.leave(&bob, &public, &bob).await.status,
        StatusCode::NOT_FOUND,
        "a public channel the person has not joined is not theirs to leave"
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn putting_in_and_leaving_tell_that_persons_open_screens_to_read_their_channels(
    pool: PgPool,
) {
    use penfii_core::ChangeNotice;

    let env = setup(pool).await;
    let bob = env.person("bob", "MEMBER").await;
    let channel = env.channel(&env.owner, "PRIVATE", None).await;
    let channel_id: Uuid = channel.parse().unwrap();

    let (member, change) =
        penfii_core::conversation::add_channel_member(&env.pool, env.owner.id, channel_id, bob.id)
            .await
            .unwrap();
    assert_eq!(member.user_id, bob.id);
    assert!(matches!(change, Some(ChangeNotice::Space { user_id }) if user_id == bob.id));
    // Repeating it puts nobody in, so there is nothing to tell.
    let (_, again) =
        penfii_core::conversation::add_channel_member(&env.pool, env.owner.id, channel_id, bob.id)
            .await
            .unwrap();
    assert!(again.is_none());

    let changes = penfii_core::conversation::leave_channel(&env.pool, bob.id, channel_id, bob.id)
        .await
        .unwrap();
    // The channel list changed, and the sockets they hold on the channel close.
    assert!(
        changes
            .iter()
            .any(|c| matches!(c, ChangeNotice::Space { user_id } if *user_id == bob.id))
    );
    assert!(
        changes
            .iter()
            .any(|c| matches!(c, ChangeNotice::Access { user_id } if *user_id == bob.id))
    );
}

impl Env {
    async fn set_role(
        &self,
        who: &Person,
        channel: &str,
        person: &Person,
        role: &str,
    ) -> common::Reply {
        self.call(
            who,
            "PATCH",
            &format!("{}/{}", Self::members_path(channel), person.id),
            Some(json!({"role": role})),
        )
        .await
    }

    async fn role_of(&self, channel: &str, person: &Person) -> String {
        let list = self
            .call(&self.owner, "GET", &Self::members_path(channel), None)
            .await;
        list.body["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["userId"] == json!(person.id))
            .map(|m| m["role"].as_str().unwrap().to_string())
            .unwrap_or_default()
    }
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_manager_hands_the_channel_to_another_member_and_leaves(pool: PgPool) {
    let env = setup(pool).await;
    let bob = env.person("bob", "MEMBER").await;
    let carol = env.person("carol", "MEMBER").await;
    let channel = env.channel(&env.owner, "PRIVATE", None).await;
    for person in [&bob, &carol] {
        assert_eq!(
            env.add(&env.owner, &channel, person).await.status,
            StatusCode::OK
        );
    }

    // A plain member cannot give roles, and cannot see their way round it.
    let refused = env.set_role(&bob, &channel, &bob, "MANAGER").await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.body);

    let promoted = env.set_role(&env.owner, &channel, &bob, "MANAGER").await;
    assert_eq!(promoted.status, StatusCode::OK, "{}", promoted.body);
    assert_eq!(promoted.body["role"], "MANAGER");
    assert_eq!(env.role_of(&channel, &bob).await, "MANAGER");

    // The first manager can now leave: the channel still has one.
    assert_eq!(
        env.leave(&env.owner, &channel, &env.owner).await.status,
        StatusCode::NO_CONTENT
    );
    // The new manager runs it: puts people in, and cannot leave while alone in the role.
    let back = env.add(&bob, &channel, &env.owner).await;
    assert_eq!(back.status, StatusCode::OK, "{}", back.body);
    let visibility = env
        .call(
            &bob,
            "PATCH",
            &format!("/v1/channels/{channel}"),
            Some(json!({"description": "run by bob"})),
        )
        .await;
    assert_eq!(visibility.status, StatusCode::OK, "{}", visibility.body);
    let stuck = env.leave(&bob, &channel, &bob).await;
    assert_eq!(stuck.status, StatusCode::CONFLICT, "{}", stuck.body);

    // Demoting works while another manager stays; the last one cannot be demoted.
    assert_eq!(
        env.set_role(&bob, &channel, &carol, "MANAGER").await.status,
        StatusCode::OK
    );
    assert_eq!(
        env.set_role(&carol, &channel, &bob, "MEMBER").await.status,
        StatusCode::OK
    );
    assert_eq!(env.role_of(&channel, &bob).await, "MEMBER");
    let last = env.set_role(&carol, &channel, &carol, "MEMBER").await;
    assert_eq!(last.status, StatusCode::CONFLICT, "{}", last.body);
    assert_eq!(last.body["code"], "last_manager");
    assert_eq!(env.role_of(&channel, &carol).await, "MANAGER");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_role_is_set_only_for_a_member_of_a_private_channel_by_its_manager(pool: PgPool) {
    let env = setup(pool).await;
    let bob = env.person("bob", "MEMBER").await;
    let carol = env.person("carol", "MEMBER").await;
    let channel = env.channel(&env.owner, "PRIVATE", None).await;
    assert_eq!(
        env.add(&env.owner, &channel, &bob).await.status,
        StatusCode::OK
    );

    // A stranger of the channel does not learn that it exists.
    assert_eq!(
        env.set_role(&carol, &channel, &bob, "MANAGER").await.status,
        StatusCode::NOT_FOUND
    );
    // The person must be in the channel already; this is not a second way in.
    assert_eq!(
        env.set_role(&env.owner, &channel, &carol, "MANAGER")
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert!(!env.can_read(&carol, &channel).await);
    // Only the two roles, and only the field the API describes.
    assert_eq!(
        env.set_role(&env.owner, &channel, &bob, "OWNER")
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    let extra = env
        .call(
            &env.owner,
            "PATCH",
            &format!("{}/{}", Env::members_path(&channel), bob.id),
            Some(json!({"role": "MANAGER", "evil": true})),
        )
        .await;
    assert_eq!(extra.status, StatusCode::BAD_REQUEST);
    // A public channel has no roles to give here.
    let public = env.channel(&env.owner, "PUBLIC", None).await;
    let refused = env
        .set_role(&env.owner, &public, &env.owner, "MEMBER")
        .await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.body);
    // A member who may invite (the channel allows it) still does not give roles.
    sqlx::query("UPDATE penfii.channels SET allow_member_invites = true WHERE id = $1")
        .bind(channel.parse::<Uuid>().unwrap())
        .execute(&env.pool)
        .await
        .unwrap();
    assert_eq!(
        env.set_role(&bob, &channel, &bob, "MANAGER").await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(env.role_of(&channel, &bob).await, "MEMBER");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn the_same_role_again_changes_nothing_and_a_change_tells_that_person(pool: PgPool) {
    use penfii_core::ChangeNotice;

    let env = setup(pool).await;
    let bob = env.person("bob", "MEMBER").await;
    let channel = env.channel(&env.owner, "PRIVATE", None).await;
    let channel_id: Uuid = channel.parse().unwrap();
    assert_eq!(
        env.add(&env.owner, &channel, &bob).await.status,
        StatusCode::OK
    );
    let events = |kind: &'static str| {
        let pool = env.pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM penfii.domain_events WHERE event_kind = $1",
            )
            .bind(kind)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };

    let (_, same) = penfii_core::conversation::set_channel_member_role(
        &env.pool,
        env.owner.id,
        channel_id,
        bob.id,
        "MEMBER",
    )
    .await
    .unwrap();
    assert!(same.is_none(), "no change, nothing to announce");
    assert_eq!(events("channel.member_role_changed").await, 0);

    let (member, change) = penfii_core::conversation::set_channel_member_role(
        &env.pool,
        env.owner.id,
        channel_id,
        bob.id,
        "MANAGER",
    )
    .await
    .unwrap();
    assert_eq!(member.role, "MANAGER");
    assert!(
        matches!(change, Some(ChangeNotice::Space { user_id }) if user_id == bob.id),
        "the person whose role changed reads their channels again"
    );
    assert_eq!(events("channel.member_role_changed").await, 1);
    let (_, again) = penfii_core::conversation::set_channel_member_role(
        &env.pool,
        env.owner.id,
        channel_id,
        bob.id,
        "MANAGER",
    )
    .await
    .unwrap();
    assert!(again.is_none());
    assert_eq!(events("channel.member_role_changed").await, 1);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn managers_who_give_up_the_role_at_once_leave_the_channel_one(pool: PgPool) {
    let env = setup(pool).await;
    let bob = env.person("bob", "MEMBER").await;
    let channel = env.channel(&env.owner, "PRIVATE", None).await;
    assert_eq!(
        env.add(&env.owner, &channel, &bob).await.status,
        StatusCode::OK
    );
    assert_eq!(
        env.set_role(&env.owner, &channel, &bob, "MANAGER")
            .await
            .status,
        StatusCode::OK
    );

    // Each demotes the other at the same moment: one must be refused, never both let through.
    for _ in 0..5 {
        let (a, b) = tokio::join!(
            env.set_role(&env.owner, &channel, &bob, "MEMBER"),
            env.set_role(&bob, &channel, &env.owner, "MEMBER"),
        );
        // Exactly one goes through. The other is refused one of two ways depending on timing: 403 when
        // the first demotion already committed (it is no longer a manager), 409 when both passed the
        // manager check and the last-manager guard stopped it. Both keep a manager, which is checked below.
        let (accepted, refused): (Vec<_>, Vec<_>) = [&a, &b]
            .into_iter()
            .partition(|response| response.status == StatusCode::OK);
        assert_eq!(accepted.len(), 1, "{} / {}", a.body, b.body);
        assert!(
            matches!(
                refused[0].status,
                StatusCode::FORBIDDEN | StatusCode::CONFLICT
            ),
            "{} / {}",
            a.body,
            b.body
        );
        let managers: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM penfii.channel_members WHERE channel_id = $1 AND role = 'MANAGER'",
        )
        .bind(channel.parse::<Uuid>().unwrap())
        .fetch_one(&env.pool)
        .await
        .unwrap();
        assert_eq!(managers, 1);
        // Put the other one back as manager for the next round.
        sqlx::query("UPDATE penfii.channel_members SET role = 'MANAGER' WHERE channel_id = $1")
            .bind(channel.parse::<Uuid>().unwrap())
            .execute(&env.pool)
            .await
            .unwrap();
    }

    // A demotion racing a departure: the manager who stays is never lost.
    let (demoted, left) = tokio::join!(
        env.set_role(&env.owner, &channel, &bob, "MEMBER"),
        env.leave(&env.owner, &channel, &env.owner),
    );
    let remaining: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.channel_members WHERE channel_id = $1 AND role = 'MANAGER'",
    )
    .bind(channel.parse::<Uuid>().unwrap())
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(remaining, 1, "{} / {}", demoted.status, left.status);
}
