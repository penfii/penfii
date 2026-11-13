//! What happens to a person's channel memberships (`channel_members`) when they leave the Workspace,
//! have their account disabled, or lose their Project role, and what they can reach after coming back.
//! They pin what removal clears (every channel membership, FEA-052), what disabling leaves alone, and
//! what a public channel without a manager still does. Losing the Project role takes the Project's
//! channel memberships with it (FEA-055; `project_participation.rs` covers each way in detail).
//!
//! Four channels cover the cases, one person (`pat`) in each: a private and a public channel of a
//! Project, and a private and a public channel that belong to no Project. A public channel gets its
//! row when the person joins it. A fifth, public Project channel that `pat` never joined is the control.

use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use crate::common::{self, Person, doc};

struct Env {
    env: common::Env,
    project: String,
    /// `[private project, public project, private global, public global]`
    channels: [String; 4],
    /// A public Project channel nobody joined.
    control: String,
}

impl std::ops::Deref for Env {
    type Target = common::Env;

    fn deref(&self) -> &common::Env {
        &self.env
    }
}

/// What a person has of one channel: whether a `channel_members` row exists, and whether they can
/// read and post.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
struct Standing {
    row: bool,
    read: bool,
    write: bool,
}

const fn standing(row: bool, read: bool, write: bool) -> Standing {
    Standing { row, read, write }
}

impl Env {
    async fn ok(&self, who: &Person, method: &str, path: &str, body: Option<serde_json::Value>) {
        let reply = self.call(who, method, path, body).await;
        assert!(
            reply.status.is_success(),
            "{method} {path}: {} {}",
            reply.status,
            reply.body
        );
    }

    async fn channel(&self, visibility: &str, project: Option<&str>, name: &str) -> String {
        let created = self
            .call(
                &self.owner,
                "POST",
                &format!("/v1/workspaces/{}/channels", self.ws),
                Some(json!({"name": name, "visibility": visibility, "projectId": project})),
            )
            .await;
        assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
        created.body["id"].as_str().unwrap().to_string()
    }

    async fn standing(&self, who: &Person, channel: &str) -> Standing {
        let channel_id: Uuid = channel.parse().unwrap();
        let row: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM penfii.channel_members WHERE channel_id = $1 AND user_id = $2)",
        )
        .bind(channel_id)
        .bind(who.id)
        .fetch_one(&self.pool)
        .await
        .unwrap();
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
        Standing { row, read, write }
    }

    async fn all(&self, who: &Person) -> [Standing; 4] {
        [
            self.standing(who, &self.channels[0]).await,
            self.standing(who, &self.channels[1]).await,
            self.standing(who, &self.channels[2]).await,
            self.standing(who, &self.channels[3]).await,
        ]
    }

    async fn sign_in_again(&self, who: &Person) -> Person {
        let (status, cookie) = common::login(&self.app, &who.email, common::MEMBER_PASSWORD).await;
        assert_eq!(status, StatusCode::OK);
        Person {
            cookie: cookie.unwrap(),
            ..who.clone()
        }
    }
}

async fn setup(pool: PgPool) -> (Env, Person) {
    let env = common::Env::new(pool).await;
    let pat = env.person("pat", "MEMBER").await;
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
        channels: Default::default(),
        control: String::new(),
    };
    env.ok(
        &env.owner,
        "PUT",
        &format!("/v1/projects/{project}/grants/{}", pat.id),
        Some(json!({"role": "EDITOR"})),
    )
    .await;
    let private_project = env.channel("PRIVATE", Some(&project), "p-private").await;
    let public_project = env.channel("PUBLIC", Some(&project), "p-public").await;
    let private_global = env.channel("PRIVATE", None, "g-private").await;
    let public_global = env.channel("PUBLIC", None, "g-public").await;
    env.control = env.channel("PUBLIC", Some(&project), "p-control").await;
    for private in [&private_project, &private_global] {
        env.ok(
            &env.owner,
            "PUT",
            &format!("/v1/channels/{private}/members/{}", pat.id),
            None,
        )
        .await;
    }
    for public in [&public_project, &public_global] {
        env.ok(&pat, "POST", &format!("/v1/channels/{public}/join"), None)
            .await;
    }
    env.channels = [
        private_project,
        public_project,
        private_global,
        public_global,
    ];
    (env, pat)
}

const FULL: Standing = standing(true, true, true);
const OPEN_WITHOUT_ROW: Standing = standing(false, true, true);
const SHUT: Standing = standing(false, false, false);

/// The baseline, and the control: a public Project channel nobody joined is readable and writable
/// through the Project alone (no row).
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_person_in_all_four_channels_and_the_control_channel(pool: PgPool) {
    let (env, pat) = setup(pool).await;
    assert_eq!(env.all(&pat).await, [FULL; 4]);
    // No row, still open: public + Project scope.
    let control = env.standing(&pat, &env.control).await;
    assert_eq!(control, OPEN_WITHOUT_ROW);
}

/// Removal from the Workspace takes every channel membership, private and public (FEA-052). What is
/// kept per person (read position, mute, favorite) and the Team and Project roles stay. A rejoin
/// brings back the Project role and so what the Project gives (its public channel), but no private
/// channel; the Workspace-wide public channel is readable and needs a join before posting.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn removal_from_the_workspace_takes_every_channel_membership_and_a_rejoin_restores_none(
    pool: PgPool,
) {
    let (env, pat) = setup(pool.clone()).await;
    // Per-person data that hangs on the channel, not on the membership row.
    let channel: Uuid = env.channels[0].parse().unwrap();
    sqlx::query("INSERT INTO penfii.channel_mutes (user_id, channel_id) VALUES ($1, $2)")
        .bind(pat.id)
        .bind(channel)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO penfii.favorites (user_id, workspace_id, kind, target_id, position)
         VALUES ($1, $2, 'CHANNEL', $3, 1)",
    )
    .bind(pat.id)
    .bind(env.ws)
    .bind(channel)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO penfii.conversation_reads (conversation_id, user_id, read_through_seq)
         VALUES ($1, $2, 1)",
    )
    .bind(channel)
    .bind(pat.id)
    .execute(&pool)
    .await
    .unwrap();
    let kept = |pool: PgPool, user: Uuid| async move {
        sqlx::query_as::<_, (i64, i64, i64, i64, i64)>(
            "SELECT (SELECT count(*) FROM penfii.channel_mutes WHERE user_id = $1),
                    (SELECT count(*) FROM penfii.favorites WHERE user_id = $1),
                    (SELECT count(*) FROM penfii.conversation_reads WHERE user_id = $1),
                    (SELECT count(*) FROM penfii.project_user_grants WHERE user_id = $1),
                    (SELECT count(*) FROM penfii.messages WHERE author_id = $1)",
        )
        .bind(user)
        .fetch_one(&pool)
        .await
        .unwrap()
    };
    // pat posts in each channel (the standing check does), so there is something to keep.
    env.all(&pat).await;
    let before = kept(pool.clone(), pat.id).await;
    assert_eq!((before.0, before.1, before.2, before.3), (1, 1, 1, 1));
    assert!(before.4 >= 2, "pat posted in the channels they joined");

    penfii_core::admin::remove_member(&pool, env.owner.id, env.ws, pat.id)
        .await
        .unwrap();
    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM penfii.channel_members WHERE user_id = $1")
            .bind(pat.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(rows, 0, "private and public memberships are all gone");
    for s in env.all(&pat).await {
        assert_eq!(s, SHUT);
    }
    assert_eq!(
        kept(pool.clone(), pat.id).await,
        before,
        "nothing else is touched"
    );
    // The audit entry says what was taken and nothing was left without a manager (the owner manages).
    let audit: serde_json::Value = sqlx::query_scalar(
        "SELECT data FROM penfii.audit_entries WHERE action_code = 'member.removed' AND target_id = $1",
    )
    .bind(pat.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(audit["clearedChannels"], json!({"private": 2, "public": 2}));
    assert_eq!(audit["channelsWithoutManager"], json!([]));

    penfii_core::membership::join_workspace_as_operator(&pool, env.ws, pat.id, "MEMBER")
        .await
        .unwrap();
    // Back: the Project role is theirs again, so the Project's public channel is open to read and
    // write. The Workspace-wide public channel can be read, posting needs a join. No private channel.
    assert_eq!(
        env.all(&pat).await,
        [SHUT, OPEN_WITHOUT_ROW, SHUT, standing(false, true, false)]
    );
    let after = kept(pool.clone(), pat.id).await;
    assert_eq!(
        (after.0, after.1, after.2, after.3),
        (before.0, before.1, before.2, before.3)
    );
    assert!(after.4 >= before.4, "no message is lost");
    // A private channel has to be given again; then it works.
    env.ok(
        &env.owner,
        "PUT",
        &format!("/v1/channels/{}/members/{}", env.channels[0], pat.id),
        None,
    )
    .await;
    assert_eq!(env.standing(&pat, &env.channels[0]).await, FULL);
}

/// A person removed before channel memberships were cleared on removal still has them. They are
/// cleared when the person comes back, so a rejoin never restores a private channel.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn memberships_left_by_an_earlier_removal_are_cleared_when_the_person_rejoins(pool: PgPool) {
    let (env, pat) = setup(pool.clone()).await;
    // The old way of removing: the state changes, nothing else.
    sqlx::query(
        "UPDATE penfii.workspace_members SET state = 'INACTIVE', deactivated_at = now()
         WHERE workspace_id = $1 AND user_id = $2",
    )
    .bind(env.ws)
    .bind(pat.id)
    .execute(&pool)
    .await
    .unwrap();
    let rows = |pool: PgPool, user: Uuid| async move {
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM penfii.channel_members WHERE user_id = $1",
        )
        .bind(user)
        .fetch_one(&pool)
        .await
        .unwrap()
    };
    assert_eq!(rows(pool.clone(), pat.id).await, 4);
    penfii_core::membership::join_workspace_as_operator(&pool, env.ws, pat.id, "MEMBER")
        .await
        .unwrap();
    assert_eq!(rows(pool.clone(), pat.id).await, 0);
    assert_eq!(env.standing(&pat, &env.channels[0]).await, SHUT);
    assert_eq!(env.standing(&pat, &env.channels[2]).await, SHUT);
}

/// Disabling an account and enabling it again: rows and membership untouched, nothing to rejoin.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn disable_and_enable_touch_no_channel_row(pool: PgPool) {
    let (env, pat) = setup(pool.clone()).await;
    penfii_core::admin::operator_set_account_disabled(&pool, &pat.email, true)
        .await
        .unwrap();
    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM penfii.channel_members WHERE user_id = $1")
            .bind(pat.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(rows, 4);
    // Disabled: no session, so no request is answered at all.
    let reply = env
        .call(
            &pat,
            "GET",
            &format!("/v1/conversations/{}/messages", env.channels[0]),
            None,
        )
        .await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    penfii_core::admin::operator_set_account_disabled(&pool, &pat.email, false)
        .await
        .unwrap();
    let pat = env.sign_in_again(&pat).await;
    assert_eq!(env.all(&pat).await, [FULL; 4]);
}

/// Losing the Project role while staying in the Workspace (FEA-055). The memberships of the Project's
/// channels, private and public, go with it, so the person reads and writes in neither; the control
/// channel (no row) closes, and channels of no Project are not affected. Getting the Project role
/// back brings no membership back: the private channel stays shut, the public one opens by the Project
/// alone (no row). The details of each way of losing participation are in `project_participation.rs`.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn losing_the_project_role_takes_the_project_channel_memberships(pool: PgPool) {
    let (env, pat) = setup(pool.clone()).await;
    let grant = format!("/v1/projects/{}/grants/{}", env.project, pat.id);
    env.ok(&env.owner, "DELETE", &grant, None).await;

    // The Project itself is closed to the person.
    let project = env
        .call(
            &pat,
            "GET",
            &format!("/v1/projects/{}/tickets", env.project),
            None,
        )
        .await;
    assert_eq!(project.status, StatusCode::NOT_FOUND);
    assert_eq!(env.standing(&pat, &env.control).await, SHUT);

    assert_eq!(
        env.all(&pat).await,
        [SHUT, SHUT, FULL, FULL],
        "the Project's channels are closed and their rows are gone"
    );

    // They are not listed as a member of the private Project channel either.
    let members = env
        .call(
            &env.owner,
            "GET",
            &format!("/v1/channels/{}/members", env.channels[0]),
            None,
        )
        .await;
    assert_eq!(members.status, StatusCode::OK, "{}", members.body);
    assert!(!members.body.to_string().contains(&pat.id.to_string()));

    env.ok(&env.owner, "PUT", &grant, Some(json!({"role": "EDITOR"})))
        .await;
    assert_eq!(
        env.all(&pat).await,
        [SHUT, OPEN_WITHOUT_ROW, FULL, FULL],
        "the private channel has to be given again; the public one follows the Project"
    );
    assert_eq!(env.standing(&pat, &env.control).await, OPEN_WITHOUT_ROW);
}

/// Removed from the Workspace after losing the Project role, then rejoined without it. The Project
/// stays closed, and now no channel of it comes back with the person: the memberships were cleared
/// on removal, so the stale rows of FEA-055 do not survive a removal.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_rejoin_without_the_project_role_brings_back_no_project_channel(pool: PgPool) {
    let (env, pat) = setup(pool.clone()).await;
    env.ok(
        &env.owner,
        "DELETE",
        &format!("/v1/projects/{}/grants/{}", env.project, pat.id),
        None,
    )
    .await;
    penfii_core::admin::remove_member(&pool, env.owner.id, env.ws, pat.id)
        .await
        .unwrap();
    penfii_core::membership::join_workspace_as_operator(&pool, env.ws, pat.id, "MEMBER")
        .await
        .unwrap();
    let project = env
        .call(
            &pat,
            "GET",
            &format!("/v1/projects/{}/tickets", env.project),
            None,
        )
        .await;
    assert_eq!(
        project.status,
        StatusCode::NOT_FOUND,
        "the Project stays closed"
    );
    assert_eq!(
        env.all(&pat).await,
        [SHUT, SHUT, SHUT, standing(false, true, false)]
    );
}

/// The invitation path of a rejoin (`join_workspace`'s Rejoined branch is shared with the operator
/// path above): a removed person who is invited and accepts again comes back to no channel, a
/// private one needs a new invitation to it, and only what is open by the Project or the Workspace
/// is readable. Their Project role is back.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn an_invitation_rejoin_brings_back_no_channel(pool: PgPool) {
    let (env, pat) = setup(pool.clone()).await;
    penfii_core::admin::remove_member(&pool, env.owner.id, env.ws, pat.id)
        .await
        .unwrap();
    let invitation = env.invite(&env.owner, &pat.email, "MEMBER").await;
    assert_eq!(
        invitation.status,
        StatusCode::CREATED,
        "{}",
        invitation.body
    );
    let accepted = common::call(
        &env.app,
        "POST",
        "/v1/invitations/accept",
        None,
        Some(json!({
            "token": invitation.body["inviteToken"],
            "displayName": "pat",
            "password": common::MEMBER_PASSWORD
        })),
    )
    .await;
    assert!(accepted.status.is_success(), "{}", accepted.body);
    let pat = env.sign_in_again(&pat).await;
    assert_eq!(
        env.all(&pat).await,
        [SHUT, OPEN_WITHOUT_ROW, SHUT, standing(false, true, false)],
        "no channel comes back with the person; the Project's public one opens by the Project role"
    );
}

/// A public channel whose only MANAGER has left the Workspace: their row is cleared by the removal
/// (FEA-052), or, for a person removed before that, it is still there (`legacy_row`). A public
/// channel without a manager is an ordinary state and behaves the same either way.
/// What this records is which actions need a MANAGER and who else can do them. Nothing here says
/// whether a public channel *should* be allowed to have no Manager.
async fn public_channel_without_a_manager(pool: PgPool, legacy_row: bool) {
    {
        let env = Env {
            env: common::Env::new(pool.clone()).await,
            project: String::new(),
            channels: Default::default(),
            control: String::new(),
        };
        let tag = "x";
        let maker = env.person(&format!("maker{tag}"), "ADMIN").await;
        let lead = env.person(&format!("lead{tag}"), "MEMBER").await;
        let pat = env.person(&format!("pat{tag}"), "MEMBER").await;
        let project = {
            let created = env
                .call(
                    &env.owner,
                    "POST",
                    &format!("/v1/workspaces/{}/projects", env.ws),
                    Some(json!({"name": "Alpha", "teamIds": []})),
                )
                .await;
            created.body["id"].as_str().unwrap().to_string()
        };
        for (who, role) in [(&maker, "ADMIN"), (&lead, "ADMIN"), (&pat, "EDITOR")] {
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
        let make = |visibility: &'static str, project: Option<String>, name: &'static str| {
            let (env, maker) = (&env, maker.clone());
            async move {
                let created = env
                    .call(
                        &maker,
                        "POST",
                        &format!("/v1/workspaces/{}/channels", env.ws),
                        Some(json!({"name": name, "visibility": visibility, "projectId": project})),
                    )
                    .await;
                assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
                created.body["id"].as_str().unwrap().to_string()
            }
        };
        let global = make("PUBLIC", None, "town").await;
        let in_project = make("PUBLIC", Some(project.clone()), "alpha-talk").await;
        // A message by pat in each, for the moderation check below.
        let mut messages = Vec::new();
        for channel in [&global, &in_project] {
            let joined = env
                .call(&pat, "POST", &format!("/v1/channels/{channel}/join"), None)
                .await;
            assert!(joined.status.is_success(), "{}", joined.body);
            let posted = env
                .call(
                    &pat,
                    "POST",
                    &format!("/v1/conversations/{channel}/messages"),
                    Some(json!({"body": doc("hi"), "bodySchemaVersion": 1, "threadRootId": null, "replyToId": null})),
                )
                .await;
            assert!(posted.status.is_success(), "{}", posted.body);
            messages.push(posted.body["id"].as_str().unwrap().to_string());
        }

        penfii_core::admin::remove_member(&pool, env.owner.id, env.ws, maker.id)
            .await
            .unwrap();
        let kept: i64 =
            sqlx::query_scalar("SELECT count(*) FROM penfii.channel_members WHERE user_id = $1")
                .bind(maker.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(kept, 0, "the removal took the manager rows");
        if legacy_row {
            for channel in [&global, &in_project] {
                sqlx::query(
                    "INSERT INTO penfii.channel_members (channel_id, user_id, role)
                     VALUES ($1, $2, 'MANAGER')",
                )
                .bind(channel.parse::<Uuid>().unwrap())
                .bind(maker.id)
                .execute(&pool)
                .await
                .unwrap();
            }
        }
        let managers: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM penfii.channel_members cm
             JOIN penfii.workspace_members wm ON wm.user_id = cm.user_id AND wm.state = 'ACTIVE'
             WHERE cm.role = 'MANAGER' AND cm.channel_id = ANY($1::uuid[])",
        )
        .bind([global.parse::<Uuid>().unwrap(), in_project.parse().unwrap()].to_vec())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            managers, 0,
            "no active Manager is left (legacy_row={legacy_row})"
        );

        // Someone with no row: reads the Workspace-wide public channel but cannot post to it until
        // they join; in the Project's public channel the Project alone is enough to post.
        let no_row_global = env.standing(&lead, &global).await;
        assert_eq!(
            (no_row_global.row, no_row_global.read, no_row_global.write),
            (false, true, false)
        );
        let no_row_project = env.standing(&lead, &in_project).await;
        assert_eq!(
            (
                no_row_project.row,
                no_row_project.read,
                no_row_project.write
            ),
            (false, true, true)
        );
        // Reading and writing are open as before (public: no Manager needed).
        for channel in [&global, &in_project] {
            let s = env.standing(&pat, channel).await;
            assert!(s.read && s.write, "{s:?}");
        }
        let status = |reply: common::Reply| reply.status;
        let patch = |who: Person, channel: String, body: serde_json::Value| {
            let env = &env;
            async move {
                status(
                    env.call(
                        &who,
                        "PATCH",
                        &format!("/v1/channels/{channel}"),
                        Some(body),
                    )
                    .await,
                )
            }
        };
        let add = |who: Person, channel: String, target: Uuid| {
            let env = &env;
            async move {
                status(
                    env.call(
                        &who,
                        "PUT",
                        &format!("/v1/channels/{channel}/members/{target}"),
                        None,
                    )
                    .await,
                )
            }
        };
        let delete = |who: Person, message: String| {
            let env = &env;
            async move {
                common::send(
                    &env.app,
                    common::request(
                        "DELETE",
                        &format!("/v1/messages/{message}"),
                        Some(&who.cookie),
                        Some(&Uuid::new_v4().to_string()),
                        None,
                    ),
                )
                .await
                .status
            }
        };
        // Settings (description, visibility): an ordinary member cannot; a Workspace OWNER/ADMIN can
        // for the Workspace-wide channel, the Project's admin for the Project's channel.
        let describe = json!({"description": "managed by hand"});
        assert_eq!(
            patch(pat.clone(), global.clone(), describe.clone()).await,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            patch(env.owner.clone(), global.clone(), describe.clone()).await,
            StatusCode::OK
        );
        assert_eq!(
            patch(pat.clone(), in_project.clone(), describe.clone()).await,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            patch(lead.clone(), in_project.clone(), describe.clone()).await,
            StatusCode::OK
        );
        // Putting people in: a member cannot (allowMemberInvites is off); the same people as above can.
        assert_eq!(
            add(pat.clone(), global.clone(), lead.id).await,
            StatusCode::FORBIDDEN
        );
        let outcome_owner_global = add(env.owner.clone(), global.clone(), lead.id).await;
        let outcome_lead_project = add(lead.clone(), in_project.clone(), pat.id).await;
        // Moderation: deleting someone else's message needs the channel MANAGER, and nobody else is
        // given that, not the Workspace owner and not the Project admin.
        let owner_deletes_global = delete(env.owner.clone(), messages[0].clone()).await;
        let lead_deletes_project = delete(lead.clone(), messages[1].clone()).await;
        let author_deletes = delete(pat.clone(), messages[0].clone()).await;
        // The Workspace owner cannot put people into the Workspace-wide channel (settings only); the
        // Project's admin can for the Project's channel (the call is repeatable, pat is in already).
        assert_eq!(outcome_owner_global, StatusCode::FORBIDDEN);
        assert_eq!(outcome_lead_project, StatusCode::OK);
        // Deleting another person's message needs the channel MANAGER: the Workspace owner and the
        // Project admin are refused. The author can always delete their own.
        assert_eq!(owner_deletes_global, StatusCode::FORBIDDEN);
        assert_eq!(lead_deletes_project, StatusCode::FORBIDDEN);
        assert_eq!(author_deletes, StatusCode::OK);
        // Public -> private by the same hands leaves a private channel nobody manages.
        let to_private = patch(
            env.owner.clone(),
            global.clone(),
            json!({"visibility": "PRIVATE"}),
        )
        .await;
        let private_managers: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM penfii.channel_members cm
             JOIN penfii.workspace_members wm ON wm.user_id = cm.user_id AND wm.state = 'ACTIVE'
             WHERE cm.role = 'MANAGER' AND cm.channel_id = $1",
        )
        .bind(global.parse::<Uuid>().unwrap())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(to_private, StatusCode::OK);
        assert_eq!(
            private_managers, 0,
            "the channel is now private and nobody manages it"
        );
    }
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_public_channel_without_a_manager_after_the_removal(pool: PgPool) {
    public_channel_without_a_manager(pool, false).await;
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_public_channel_without_a_manager_with_a_legacy_row_left(pool: PgPool) {
    public_channel_without_a_manager(pool, true).await;
}
