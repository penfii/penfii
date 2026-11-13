//! Joining a Workspace: one rule for an accepted invitation and for the operator.
//! `penfii_core::membership::join_workspace` is what `admin-cli` uses; the invitation flow calls the
//! same function, so these tests hold both to the same behaviour.

use axum::http::StatusCode;
use penfii_core::CoreError;
use penfii_core::identity::{self, BootstrapInput};
use penfii_core::membership::{JoinOutcome, join_workspace_as_operator};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::common::{self, MEMBER_PASSWORD as PASSWORD, call};

/// An account that exists, with a Workspace of its own (what `create_account` makes).
async fn account(pool: &PgPool, name: &str) -> (Uuid, String) {
    let email = format!("{name}@penfii.test");
    let made = identity::create_account(
        pool,
        BootstrapInput {
            email: email.clone(),
            password: PASSWORD.into(),
            display_name: name.into(),
            workspace_name: format!("{name} space"),
            time_zone: "Asia/Seoul".into(),
        },
    )
    .await
    .expect("account");
    (made.user_id, email)
}

async fn membership(pool: &PgPool, ws: Uuid, user: Uuid) -> Option<(String, String, bool)> {
    sqlx::query_as(
        "SELECT role, state, deactivated_at IS NOT NULL FROM penfii.workspace_members
         WHERE workspace_id = $1 AND user_id = $2",
    )
    .bind(ws)
    .bind(user)
    .fetch_optional(pool)
    .await
    .unwrap()
}

/// `(actor, data)` of every `member.joined` entry about `user`, oldest first.
async fn joined_entries(pool: &PgPool, ws: Uuid, user: Uuid) -> Vec<(Option<Uuid>, Value)> {
    sqlx::query_as(
        "SELECT actor_user_id, data FROM penfii.audit_entries
         WHERE workspace_id = $1 AND action_code = 'member.joined' AND target_id = $2
         ORDER BY occurred_at, id",
    )
    .bind(ws)
    .bind(user)
    .fetch_all(pool)
    .await
    .unwrap()
}

fn is_conflict(error: &CoreError, wanted: &str) -> bool {
    matches!(error, CoreError::Conflict { code, .. } if *code == wanted)
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn the_operator_joins_a_new_member_with_any_role_and_it_is_audited(pool: PgPool) {
    let env = common::Env::new(pool.clone()).await;
    for (name, role) in [("m", "MEMBER"), ("a", "ADMIN"), ("o", "OWNER")] {
        let (user, _) = account(&pool, name).await;
        let outcome = join_workspace_as_operator(&pool, env.ws, user, role)
            .await
            .unwrap();
        assert_eq!(outcome, JoinOutcome::Joined);
        assert!(outcome.changed());
        assert_eq!(
            membership(&pool, env.ws, user).await,
            Some((role.into(), "ACTIVE".into(), false))
        );
        // No actor (nobody is signed in), and the entry says how the person came in.
        let entries = joined_entries(&pool, env.ws, user).await;
        assert_eq!(entries.len(), 1, "{entries:?}");
        assert_eq!(entries[0].0, None);
        assert_eq!(entries[0].1["via"], "admin-cli");
        assert_eq!(entries[0].1["role"], role);
        assert_eq!(entries[0].1["rejoined"], false);
        assert!(entries[0].1.get("invitationId").is_none());
    }
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_role_that_does_not_exist_and_a_missing_workspace_or_account_are_refused(pool: PgPool) {
    let env = common::Env::new(pool.clone()).await;
    let (user, _) = account(&pool, "p").await;
    let bad_role = join_workspace_as_operator(&pool, env.ws, user, "BOSS")
        .await
        .unwrap_err();
    assert!(matches!(bad_role, CoreError::Validation(_)), "{bad_role:?}");
    let no_workspace = join_workspace_as_operator(&pool, Uuid::new_v4(), user, "MEMBER")
        .await
        .unwrap_err();
    assert!(
        matches!(no_workspace, CoreError::NotFound),
        "{no_workspace:?}"
    );
    let no_account = join_workspace_as_operator(&pool, env.ws, Uuid::new_v4(), "MEMBER")
        .await
        .unwrap_err();
    assert!(matches!(no_account, CoreError::NotFound), "{no_account:?}");
    assert_eq!(membership(&pool, env.ws, user).await, None);
    assert!(joined_entries(&pool, env.ws, user).await.is_empty());
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn an_active_member_is_a_duplicate_that_changes_neither_role_nor_audit_log(pool: PgPool) {
    let env = common::Env::new(pool.clone()).await;
    let (user, _) = account(&pool, "dup").await;
    join_workspace_as_operator(&pool, env.ws, user, "MEMBER")
        .await
        .unwrap();
    let again = join_workspace_as_operator(&pool, env.ws, user, "ADMIN")
        .await
        .unwrap();
    assert_eq!(
        again,
        JoinOutcome::AlreadyMember {
            role: "MEMBER".into()
        }
    );
    assert!(!again.changed());
    assert_eq!(
        membership(&pool, env.ws, user).await,
        Some(("MEMBER".into(), "ACTIVE".into(), false))
    );
    assert_eq!(joined_entries(&pool, env.ws, user).await.len(), 1);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_removed_member_comes_back_active_with_the_new_role(pool: PgPool) {
    let env = common::Env::new(pool.clone()).await;
    let (user, _) = account(&pool, "back").await;
    join_workspace_as_operator(&pool, env.ws, user, "ADMIN")
        .await
        .unwrap();
    penfii_core::admin::remove_member(&pool, env.owner.id, env.ws, user)
        .await
        .unwrap();
    assert_eq!(
        membership(&pool, env.ws, user).await,
        Some(("ADMIN".into(), "INACTIVE".into(), true))
    );

    let outcome = join_workspace_as_operator(&pool, env.ws, user, "MEMBER")
        .await
        .unwrap();
    assert_eq!(outcome, JoinOutcome::Rejoined);
    assert!(outcome.changed());
    // The earlier role (ADMIN) is not brought back; the removal mark is gone.
    assert_eq!(
        membership(&pool, env.ws, user).await,
        Some(("MEMBER".into(), "ACTIVE".into(), false))
    );
    let entries = joined_entries(&pool, env.ws, user).await;
    assert_eq!(entries.len(), 2, "{entries:?}");
    assert_eq!(entries[1].1["rejoined"], true);
    assert_eq!(entries[1].1["role"], "MEMBER");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_disabled_account_does_not_join_until_it_is_enabled(pool: PgPool) {
    let env = common::Env::new(pool.clone()).await;
    let (fresh, fresh_email) = account(&pool, "locked").await;
    penfii_core::admin::operator_set_account_disabled(&pool, &fresh_email, true)
        .await
        .unwrap();
    let refused = join_workspace_as_operator(&pool, env.ws, fresh, "MEMBER")
        .await
        .unwrap_err();
    assert!(is_conflict(&refused, "account_disabled"), "{refused:?}");
    assert_eq!(membership(&pool, env.ws, fresh).await, None);
    assert!(joined_entries(&pool, env.ws, fresh).await.is_empty());

    // A removed member whose account was disabled meanwhile stays removed.
    let (gone, gone_email) = account(&pool, "gone").await;
    join_workspace_as_operator(&pool, env.ws, gone, "MEMBER")
        .await
        .unwrap();
    penfii_core::admin::remove_member(&pool, env.owner.id, env.ws, gone)
        .await
        .unwrap();
    penfii_core::admin::operator_set_account_disabled(&pool, &gone_email, true)
        .await
        .unwrap();
    let refused = join_workspace_as_operator(&pool, env.ws, gone, "MEMBER")
        .await
        .unwrap_err();
    assert!(is_conflict(&refused, "account_disabled"), "{refused:?}");
    assert_eq!(
        membership(&pool, env.ws, gone).await,
        Some(("MEMBER".into(), "INACTIVE".into(), true))
    );

    // Enabled, both can join.
    for (user, email) in [(fresh, &fresh_email), (gone, &gone_email)] {
        penfii_core::admin::operator_set_account_disabled(&pool, email, false)
            .await
            .unwrap();
        assert!(
            join_workspace_as_operator(&pool, env.ws, user, "MEMBER")
                .await
                .unwrap()
                .changed()
        );
    }
}

async fn accept(env: &common::Env, invitation: &Value, name: &str) -> (StatusCode, Value) {
    let reply = call(
        &env.app,
        "POST",
        "/v1/invitations/accept",
        None,
        Some(
            json!({"token": invitation["inviteToken"], "displayName": name, "password": PASSWORD}),
        ),
    )
    .await;
    (reply.status, reply.body)
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn an_invitation_joins_through_the_same_rule_and_audits_the_person_as_actor(pool: PgPool) {
    let env = common::Env::new(pool.clone()).await;
    // A new account.
    let invitation = env.invite(&env.owner, "new@penfii.test", "ADMIN").await;
    assert_eq!(
        invitation.status,
        StatusCode::CREATED,
        "{}",
        invitation.body
    );
    let (status, body) = accept(&env, &invitation.body, "new").await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let user: Uuid = body["user"]["id"].as_str().unwrap().parse().unwrap();
    assert_eq!(
        membership(&pool, env.ws, user).await,
        Some(("ADMIN".into(), "ACTIVE".into(), false))
    );
    let entries = joined_entries(&pool, env.ws, user).await;
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert_eq!(entries[0].0, Some(user));
    assert_eq!(entries[0].1["invitationId"], invitation.body["id"]);
    assert_eq!(entries[0].1["rejoined"], false);
    assert!(entries[0].1.get("via").is_none());

    // The same person, removed and invited again: back with the invited role.
    penfii_core::admin::remove_member(&pool, env.owner.id, env.ws, user)
        .await
        .unwrap();
    let invitation = env.invite(&env.owner, "new@penfii.test", "MEMBER").await;
    let (status, body) = accept(&env, &invitation.body, "new").await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(
        membership(&pool, env.ws, user).await,
        Some(("MEMBER".into(), "ACTIVE".into(), false))
    );
    let entries = joined_entries(&pool, env.ws, user).await;
    assert_eq!(entries.len(), 2, "{entries:?}");
    assert_eq!(entries[1].1["rejoined"], true);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn an_invitation_to_someone_who_already_joined_is_accepted_and_changes_nothing(pool: PgPool) {
    let env = common::Env::new(pool.clone()).await;
    let (user, email) = account(&pool, "early").await;
    // The invitation is made first; the operator adds the person as an admin before it is used.
    let invitation = env.invite(&env.owner, &email, "MEMBER").await;
    assert_eq!(
        invitation.status,
        StatusCode::CREATED,
        "{}",
        invitation.body
    );
    join_workspace_as_operator(&pool, env.ws, user, "ADMIN")
        .await
        .unwrap();

    let (status, body) = accept(&env, &invitation.body, "early").await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(
        membership(&pool, env.ws, user).await,
        Some(("ADMIN".into(), "ACTIVE".into(), false))
    );
    // Only the operator's join is audited; the duplicate is not.
    assert_eq!(joined_entries(&pool, env.ws, user).await.len(), 1);
    let accepted: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.workspace_invitations
         WHERE email_key = $1 AND accepted_user_id = $2",
    )
    .bind(&email)
    .bind(user)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(accepted, 1);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_disabled_account_cannot_accept_an_invitation_either(pool: PgPool) {
    let env = common::Env::new(pool.clone()).await;
    let (user, email) = account(&pool, "off").await;
    join_workspace_as_operator(&pool, env.ws, user, "MEMBER")
        .await
        .unwrap();
    penfii_core::admin::remove_member(&pool, env.owner.id, env.ws, user)
        .await
        .unwrap();
    penfii_core::admin::operator_set_account_disabled(&pool, &email, true)
        .await
        .unwrap();
    let invitation = env.invite(&env.owner, &email, "MEMBER").await;
    assert_eq!(
        invitation.status,
        StatusCode::CREATED,
        "{}",
        invitation.body
    );

    // The answer is the one for a wrong password: it does not tell the account is disabled.
    let (status, body) = accept(&env, &invitation.body, "off").await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "account_exists");
    assert_eq!(
        membership(&pool, env.ws, user).await,
        Some(("MEMBER".into(), "INACTIVE".into(), true))
    );
}

// Concurrent joins of the same account: the single INSERT .. ON CONFLICT statement decides, so one
// request changes the membership and the others see it as it is.

const ROLES: [&str; 6] = ["MEMBER", "ADMIN", "OWNER", "MEMBER", "ADMIN", "OWNER"];

async fn join_all(
    pool: &PgPool,
    ws: Uuid,
    user: Uuid,
    roles: &[&'static str],
) -> Vec<Result<JoinOutcome, CoreError>> {
    let mut set = tokio::task::JoinSet::new();
    for role in roles.iter().copied() {
        let pool = pool.clone();
        set.spawn(async move { join_workspace_as_operator(&pool, ws, user, role).await });
    }
    let mut results = Vec::new();
    while let Some(done) = set.join_next().await {
        results.push(done.unwrap());
    }
    results
}

/// One request changed the membership; every other one reported the role that is stored.
fn assert_one_winner(results: &[Result<JoinOutcome, CoreError>], stored_role: &str, changed: &str) {
    let outcomes: Vec<&JoinOutcome> = results
        .iter()
        .map(|r| r.as_ref().expect("no join fails"))
        .collect();
    let winners = outcomes.iter().filter(|o| o.changed()).count();
    assert_eq!(winners, 1, "{outcomes:?}");
    assert_eq!(
        outcomes
            .iter()
            .filter(|o| matches!(o, JoinOutcome::Joined | JoinOutcome::Rejoined))
            .count(),
        1
    );
    assert!(
        outcomes.iter().any(|o| matches!(
            (changed, o),
            ("joined", JoinOutcome::Joined) | ("rejoined", JoinOutcome::Rejoined)
        )),
        "{outcomes:?}"
    );
    for outcome in outcomes {
        if let JoinOutcome::AlreadyMember { role } = outcome {
            assert_eq!(role, stored_role, "a duplicate reports the stored role");
        }
    }
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn joins_at_once_make_one_membership_with_one_role_and_one_audit_entry(pool: PgPool) {
    let env = common::Env::new(pool.clone()).await;
    for round in 0..8 {
        let (user, _) = account(&pool, &format!("race{round}")).await;
        let results = join_all(&pool, env.ws, user, &ROLES).await;

        let rows: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM penfii.workspace_members WHERE workspace_id = $1 AND user_id = $2",
        )
        .bind(env.ws)
        .bind(user)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(rows, 1);
        let (role, state, _) = membership(&pool, env.ws, user).await.unwrap();
        assert_eq!(state, "ACTIVE");
        assert_one_winner(&results, &role, "joined");
        // The role that is stored is the winner's, and the one entry says so.
        let entries = joined_entries(&pool, env.ws, user).await;
        assert_eq!(entries.len(), 1, "{entries:?}");
        assert_eq!(entries[0].1["role"], role.as_str());
        assert_eq!(entries[0].1["rejoined"], false);
    }
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn rejoins_at_once_revive_the_membership_once(pool: PgPool) {
    let env = common::Env::new(pool.clone()).await;
    for round in 0..8 {
        let (user, _) = account(&pool, &format!("back{round}")).await;
        join_workspace_as_operator(&pool, env.ws, user, "ADMIN")
            .await
            .unwrap();
        penfii_core::admin::remove_member(&pool, env.owner.id, env.ws, user)
            .await
            .unwrap();

        let results = join_all(&pool, env.ws, user, &ROLES).await;

        let (role, state, removed) = membership(&pool, env.ws, user).await.unwrap();
        assert_eq!((state.as_str(), removed), ("ACTIVE", false));
        assert_one_winner(&results, &role, "rejoined");
        // The first join and exactly one rejoin.
        let entries = joined_entries(&pool, env.ws, user).await;
        assert_eq!(entries.len(), 2, "{entries:?}");
        assert_eq!(entries[1].1["rejoined"], true);
        assert_eq!(entries[1].1["role"], role.as_str());
    }
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn joins_at_once_with_an_active_member_change_nothing(pool: PgPool) {
    let env = common::Env::new(pool.clone()).await;
    let (user, _) = account(&pool, "steady").await;
    join_workspace_as_operator(&pool, env.ws, user, "ADMIN")
        .await
        .unwrap();
    let results = join_all(&pool, env.ws, user, &ROLES).await;
    for result in results {
        assert_eq!(
            result.unwrap(),
            JoinOutcome::AlreadyMember {
                role: "ADMIN".into()
            }
        );
    }
    assert_eq!(
        membership(&pool, env.ws, user).await,
        Some(("ADMIN".into(), "ACTIVE".into(), false))
    );
    assert_eq!(joined_entries(&pool, env.ws, user).await.len(), 1);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_join_racing_the_account_being_disabled_is_either_refused_or_complete(pool: PgPool) {
    let env = common::Env::new(pool.clone()).await;
    for round in 0..8 {
        let (user, email) = account(&pool, &format!("off{round}")).await;
        let joiner = {
            let pool = pool.clone();
            let ws = env.ws;
            tokio::spawn(async move { join_workspace_as_operator(&pool, ws, user, "MEMBER").await })
        };
        let disabler = {
            let pool = pool.clone();
            tokio::spawn(async move {
                penfii_core::admin::operator_set_account_disabled(&pool, &email, true).await
            })
        };
        let joined = joiner.await.unwrap();
        disabler.await.unwrap().unwrap();
        // Whichever came first, the books agree: a membership exists exactly when the join succeeded
        // and it is audited once; a refused join leaves nothing behind.
        match joined {
            Ok(JoinOutcome::Joined) => {
                assert!(membership(&pool, env.ws, user).await.is_some());
                assert_eq!(joined_entries(&pool, env.ws, user).await.len(), 1);
            }
            Err(error) => {
                assert!(is_conflict(&error, "account_disabled"), "{error:?}");
                assert_eq!(membership(&pool, env.ws, user).await, None);
                assert!(joined_entries(&pool, env.ws, user).await.is_empty());
            }
            other => panic!("{other:?}"),
        }
        // Once the account is disabled no further join gets in.
        let late = join_workspace_as_operator(&pool, env.ws, user, "ADMIN").await;
        match late {
            Err(error) => assert!(is_conflict(&error, "account_disabled"), "{error:?}"),
            Ok(JoinOutcome::AlreadyMember { .. }) => {}
            Ok(other) => panic!("{other:?}"),
        }
    }
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn an_invitation_and_the_operator_joining_at_once_audit_one_join(pool: PgPool) {
    let env = common::Env::new(pool.clone()).await;
    for round in 0..6 {
        let (user, email) = account(&pool, &format!("both{round}")).await;
        let invitation = env.invite(&env.owner, &email, "MEMBER").await;
        assert_eq!(
            invitation.status,
            StatusCode::CREATED,
            "{}",
            invitation.body
        );
        let accepter = {
            let app = env.app.clone();
            let token = invitation.body["inviteToken"].clone();
            tokio::spawn(async move {
                call(
                    &app,
                    "POST",
                    "/v1/invitations/accept",
                    None,
                    Some(json!({"token": token, "displayName": "both", "password": PASSWORD})),
                )
                .await
                .status
            })
        };
        let operator = {
            let pool = pool.clone();
            let ws = env.ws;
            tokio::spawn(async move { join_workspace_as_operator(&pool, ws, user, "ADMIN").await })
        };
        assert_eq!(accepter.await.unwrap(), StatusCode::CREATED);
        let outcome = operator.await.unwrap().unwrap();

        let (role, state, _) = membership(&pool, env.ws, user).await.unwrap();
        assert_eq!(state, "ACTIVE");
        let entries = joined_entries(&pool, env.ws, user).await;
        assert_eq!(entries.len(), 1, "{entries:?}");
        // Whoever got in first decided the role; the other left it alone.
        assert_eq!(entries[0].1["role"], role.as_str());
        match outcome {
            JoinOutcome::Joined => assert_eq!(entries[0].1["via"], "admin-cli"),
            JoinOutcome::AlreadyMember { role: seen } => {
                assert_eq!(seen, role);
                assert!(entries[0].1.get("invitationId").is_some());
            }
            other => panic!("{other:?}"),
        }
    }
}

// The operator's commands: create-user, workspace-add-member, workspace-set-role, workspace-list.

use penfii_core::membership::{operator_add_member, operator_create_member};

async fn count(pool: &PgPool, table: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT count(*) FROM penfii.{table}"))
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn user_by_email(pool: &PgPool, email: &str) -> Option<Uuid> {
    sqlx::query_scalar("SELECT id FROM penfii.users WHERE email_key = $1")
        .bind(email)
        .fetch_optional(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn create_user_makes_the_account_and_joins_the_workspace_without_a_workspace_of_its_own(
    pool: PgPool,
) {
    let env = common::Env::new(pool.clone()).await;
    let workspaces = count(&pool, "workspaces").await;
    let made = operator_create_member(
        &pool,
        env.ws,
        "New.Person@Penfii.Test",
        PASSWORD,
        "New Person",
        "ADMIN",
    )
    .await
    .unwrap();
    assert_eq!(made.outcome, JoinOutcome::Joined);
    assert_eq!(made.workspace_id, env.ws);
    // The e-mail is stored the way every account's is, and the person can sign in with the password.
    assert_eq!(
        user_by_email(&pool, "new.person@penfii.test").await,
        Some(made.user_id)
    );
    assert_eq!(
        membership(&pool, env.ws, made.user_id).await,
        Some(("ADMIN".into(), "ACTIVE".into(), false))
    );
    assert_eq!(count(&pool, "workspaces").await, workspaces);
    let other: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.workspace_members WHERE user_id = $1 AND workspace_id <> $2",
    )
    .bind(made.user_id)
    .bind(env.ws)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(other, 0, "no workspace of its own");
    let signed_in = common::login(&env.app, "new.person@penfii.test", PASSWORD).await;
    assert_eq!(signed_in.0, StatusCode::OK);
    // The same entry as any other join, through admin-cli.
    let entries = joined_entries(&pool, env.ws, made.user_id).await;
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert_eq!(entries[0].0, None);
    assert_eq!(entries[0].1["via"], "admin-cli");
    assert_eq!(entries[0].1["role"], "ADMIN");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn create_user_that_cannot_complete_leaves_nothing_behind(pool: PgPool) {
    let env = common::Env::new(pool.clone()).await;
    let (_, taken) = account(&pool, "taken").await;
    let users = count(&pool, "users").await;

    // An e-mail that has an account, in any spelling.
    let error = operator_create_member(
        &pool,
        env.ws,
        &taken.to_uppercase(),
        PASSWORD,
        "T",
        "MEMBER",
    )
    .await
    .unwrap_err();
    assert!(is_conflict(&error, "account_exists"), "{error:?}");
    // A weak password, a name that is empty, a role that does not exist, a Workspace that does not.
    let weak = operator_create_member(&pool, env.ws, "weak@penfii.test", "short", "W", "MEMBER")
        .await
        .unwrap_err();
    assert!(matches!(weak, CoreError::Validation(_)), "{weak:?}");
    let nameless = operator_create_member(
        &pool,
        env.ws,
        "nameless@penfii.test",
        PASSWORD,
        " ",
        "MEMBER",
    )
    .await
    .unwrap_err();
    assert!(matches!(nameless, CoreError::Validation(_)), "{nameless:?}");
    let boss = operator_create_member(&pool, env.ws, "boss@penfii.test", PASSWORD, "B", "BOSS")
        .await
        .unwrap_err();
    assert!(matches!(boss, CoreError::Validation(_)), "{boss:?}");
    let nowhere = operator_create_member(
        &pool,
        Uuid::new_v4(),
        "nowhere@penfii.test",
        PASSWORD,
        "N",
        "MEMBER",
    )
    .await
    .unwrap_err();
    assert!(matches!(nowhere, CoreError::NotFound), "{nowhere:?}");

    assert_eq!(count(&pool, "users").await, users);
    assert_eq!(user_by_email(&pool, "nowhere@penfii.test").await, None);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn add_member_follows_the_join_rule_for_an_account_found_by_e_mail(pool: PgPool) {
    let env = common::Env::new(pool.clone()).await;
    let (user, email) = account(&pool, "guest").await;

    // Not an account yet: the operator is pointed at create-user.
    let missing = operator_add_member(&pool, env.ws, "ghost@penfii.test", "MEMBER")
        .await
        .unwrap_err();
    assert!(
        matches!(&missing, CoreError::Rule { code, .. } if *code == "account_not_found"),
        "{missing:?}"
    );

    let first = operator_add_member(&pool, env.ws, &email.to_uppercase(), "MEMBER")
        .await
        .unwrap();
    assert_eq!((first.user_id, first.outcome), (user, JoinOutcome::Joined));
    // Already in: reported with the stored role, nothing changes.
    let again = operator_add_member(&pool, env.ws, &email, "OWNER")
        .await
        .unwrap();
    assert_eq!(
        again.outcome,
        JoinOutcome::AlreadyMember {
            role: "MEMBER".into()
        }
    );
    // Removed: back with the role given now.
    penfii_core::admin::remove_member(&pool, env.owner.id, env.ws, user)
        .await
        .unwrap();
    let back = operator_add_member(&pool, env.ws, &email, "ADMIN")
        .await
        .unwrap();
    assert_eq!(back.outcome, JoinOutcome::Rejoined);
    assert_eq!(
        membership(&pool, env.ws, user).await,
        Some(("ADMIN".into(), "ACTIVE".into(), false))
    );
    // Disabled: enable first. An unknown Workspace is not found.
    penfii_core::admin::remove_member(&pool, env.owner.id, env.ws, user)
        .await
        .unwrap();
    penfii_core::admin::operator_set_account_disabled(&pool, &email, true)
        .await
        .unwrap();
    let locked = operator_add_member(&pool, env.ws, &email, "MEMBER")
        .await
        .unwrap_err();
    assert!(is_conflict(&locked, "account_disabled"), "{locked:?}");
    let nowhere = operator_add_member(&pool, Uuid::new_v4(), &email, "MEMBER")
        .await
        .unwrap_err();
    assert!(matches!(nowhere, CoreError::NotFound), "{nowhere:?}");
    assert_eq!(joined_entries(&pool, env.ws, user).await.len(), 2);
}

async fn role_entries(pool: &PgPool, ws: Uuid, user: Uuid) -> Vec<(Option<Uuid>, Value)> {
    sqlx::query_as(
        "SELECT actor_user_id, data FROM penfii.audit_entries
         WHERE workspace_id = $1 AND action_code = 'member.role_changed' AND target_id = $2
         ORDER BY occurred_at, id",
    )
    .bind(ws)
    .bind(user)
    .fetch_all(pool)
    .await
    .unwrap()
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn set_role_changes_an_active_members_role_and_audits_it_through_admin_cli(pool: PgPool) {
    let env = common::Env::new(pool.clone()).await;
    let (user, email) = account(&pool, "promoted").await;
    join_workspace_as_operator(&pool, env.ws, user, "MEMBER")
        .await
        .unwrap();

    let change = penfii_core::admin::operator_set_member_role(&pool, env.ws, &email, "ADMIN")
        .await
        .unwrap();
    assert_eq!(
        (
            change.user_id,
            change.changed,
            change.before.as_str(),
            change.after.as_str()
        ),
        (user, true, "MEMBER", "ADMIN")
    );
    assert_eq!(
        membership(&pool, env.ws, user).await,
        Some(("ADMIN".into(), "ACTIVE".into(), false))
    );
    let entries = role_entries(&pool, env.ws, user).await;
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert_eq!(entries[0].0, None);
    assert_eq!(entries[0].1["before"], "MEMBER");
    assert_eq!(entries[0].1["after"], "ADMIN");
    assert_eq!(entries[0].1["via"], "admin-cli");

    // The same role again changes nothing and is not audited.
    let same = penfii_core::admin::operator_set_member_role(&pool, env.ws, &email, "ADMIN")
        .await
        .unwrap();
    assert!(!same.changed);
    assert_eq!(role_entries(&pool, env.ws, user).await.len(), 1);

    // The operator's change and the web's agree: an owner sees the role the operator set.
    let reply = env
        .call(
            &env.owner,
            "GET",
            &format!("/v1/workspaces/{}/members", env.ws),
            None,
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let listed = reply.body;
    let row = listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["userId"] == json!(user))
        .unwrap()
        .clone();
    assert_eq!(row["role"], "ADMIN");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn set_role_refuses_what_is_not_an_active_member_and_keeps_the_last_owner(pool: PgPool) {
    let env = common::Env::new(pool.clone()).await;
    let owner_email = common::OWNER_EMAIL;
    let (outsider, outsider_email) = account(&pool, "outside").await;

    let unknown =
        penfii_core::admin::operator_set_member_role(&pool, env.ws, "ghost@penfii.test", "ADMIN")
            .await
            .unwrap_err();
    assert!(
        matches!(&unknown, CoreError::Rule { code, .. } if *code == "account_not_found"),
        "{unknown:?}"
    );
    let bad_role = penfii_core::admin::operator_set_member_role(&pool, env.ws, owner_email, "BOSS")
        .await
        .unwrap_err();
    assert!(matches!(bad_role, CoreError::Validation(_)), "{bad_role:?}");
    let nowhere =
        penfii_core::admin::operator_set_member_role(&pool, Uuid::new_v4(), owner_email, "ADMIN")
            .await
            .unwrap_err();
    assert!(matches!(nowhere, CoreError::NotFound), "{nowhere:?}");
    // Never a member, and a removed member: neither has a role to change. Joining comes first.
    for _ in 0..2 {
        let error =
            penfii_core::admin::operator_set_member_role(&pool, env.ws, &outsider_email, "ADMIN")
                .await
                .unwrap_err();
        assert!(
            matches!(&error, CoreError::Rule { code, .. } if *code == "not_a_member"),
            "{error:?}"
        );
        join_workspace_as_operator(&pool, env.ws, outsider, "MEMBER")
            .await
            .unwrap();
        penfii_core::admin::remove_member(&pool, env.owner.id, env.ws, outsider)
            .await
            .unwrap();
    }

    // The only owner stays an owner, however the operator asks.
    let last = penfii_core::admin::operator_set_member_role(&pool, env.ws, owner_email, "ADMIN")
        .await
        .unwrap_err();
    assert!(is_conflict(&last, "last_owner"), "{last:?}");
    assert_eq!(
        membership(&pool, env.ws, env.owner.id).await.map(|m| m.0),
        Some("OWNER".into())
    );
    // With a second owner (made by the operator, which a web owner could also do), it is allowed.
    let (second, second_email) = account(&pool, "second").await;
    join_workspace_as_operator(&pool, env.ws, second, "MEMBER")
        .await
        .unwrap();
    penfii_core::admin::operator_set_member_role(&pool, env.ws, &second_email, "OWNER")
        .await
        .unwrap();
    let demoted = penfii_core::admin::operator_set_member_role(&pool, env.ws, owner_email, "ADMIN")
        .await
        .unwrap();
    assert!(demoted.changed);
    // ... and now the other one is the last.
    let last = penfii_core::admin::operator_set_member_role(&pool, env.ws, &second_email, "MEMBER")
        .await
        .unwrap_err();
    assert!(is_conflict(&last, "last_owner"), "{last:?}");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn two_owners_demoted_at_once_leave_one(pool: PgPool) {
    let env = common::Env::new(pool.clone()).await;
    let (second, second_email) = account(&pool, "twin").await;
    join_workspace_as_operator(&pool, env.ws, second, "OWNER")
        .await
        .unwrap();
    let owner_email = common::OWNER_EMAIL.to_string();
    let a = {
        let pool = pool.clone();
        let ws = env.ws;
        tokio::spawn(async move {
            penfii_core::admin::operator_set_member_role(&pool, ws, &owner_email, "MEMBER").await
        })
    };
    let b = {
        let pool = pool.clone();
        let ws = env.ws;
        tokio::spawn(async move {
            penfii_core::admin::operator_set_member_role(&pool, ws, &second_email, "MEMBER").await
        })
    };
    let (a, b) = (a.await.unwrap(), b.await.unwrap());
    assert_eq!(
        [a.is_ok(), b.is_ok()].iter().filter(|ok| **ok).count(),
        1,
        "{a:?} {b:?}"
    );
    let owners: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.workspace_members
         WHERE workspace_id = $1 AND state = 'ACTIVE' AND role = 'OWNER'",
    )
    .bind(env.ws)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(owners, 1);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn workspace_list_shows_each_workspace_with_its_active_members_and_owners(pool: PgPool) {
    let env = common::Env::new(pool.clone()).await;
    let (member, _) = account(&pool, "listed").await;
    join_workspace_as_operator(&pool, env.ws, member, "MEMBER")
        .await
        .unwrap();
    let (gone, _) = account(&pool, "left").await;
    join_workspace_as_operator(&pool, env.ws, gone, "MEMBER")
        .await
        .unwrap();
    penfii_core::admin::remove_member(&pool, env.owner.id, env.ws, gone)
        .await
        .unwrap();

    let listed = penfii_core::admin::operator_list_workspaces(&pool)
        .await
        .unwrap();
    // Every workspace, including the ones `create_account` made for the helper accounts.
    assert_eq!(listed.len() as i64, count(&pool, "workspaces").await);
    let ours = listed.iter().find(|w| w.id == env.ws).unwrap();
    assert_eq!(ours.name, "W");
    assert_eq!(ours.time_zone, "Asia/Seoul");
    assert_eq!(ours.active_members, 2, "the removed member is not counted");
    assert_eq!(ours.disabled_members, 0);
    assert_eq!(ours.owners, [common::OWNER_EMAIL.to_string()]);
    assert!(ours.disabled_owners.is_empty());
    // The oldest comes first.
    assert_eq!(listed[0].id, env.ws);
}

/// A disabled account cannot act, so the listing says how many members and which owners are
/// disabled; the counts that guard the owners (FEA-054) are not changed by it.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn workspace_list_marks_the_disabled_members_and_owners(pool: PgPool) {
    let env = common::Env::new(pool.clone()).await;
    let (second_owner, second_email) = account(&pool, "second-owner").await;
    join_workspace_as_operator(&pool, env.ws, second_owner, "OWNER")
        .await
        .unwrap();
    let (member, member_email) = account(&pool, "quiet").await;
    join_workspace_as_operator(&pool, env.ws, member, "MEMBER")
        .await
        .unwrap();
    for email in [&second_email, &member_email] {
        penfii_core::admin::operator_set_account_disabled(&pool, email, true)
            .await
            .unwrap();
    }
    let listed = penfii_core::admin::operator_list_workspaces(&pool)
        .await
        .unwrap();
    let ours = listed.iter().find(|w| w.id == env.ws).unwrap();
    assert_eq!(
        ours.active_members, 3,
        "a disabled account is still a member"
    );
    assert_eq!(ours.disabled_members, 2);
    assert_eq!(ours.owners.len(), 2);
    assert_eq!(ours.disabled_owners, [second_email]);
}

/// Coming back to a Workspace restores the person's place in the organisation: their Teams, what a
/// Team gives them in a Project, and their direct Project roles. (Private channels are the exception,
/// see FEA-052: their memberships are to be cleared on removal and are not restored.) While the
/// membership is removed, none of it gives any access.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_rejoin_restores_team_and_project_access_while_a_removal_closes_it(pool: PgPool) {
    let env = common::Env::new(pool.clone()).await;
    let by_team = env.person("byteam", "MEMBER").await;
    let direct = env.person("direct", "MEMBER").await;
    let ok = |reply: common::Reply| {
        assert!(reply.status.is_success(), "{} {}", reply.status, reply.body);
        reply.body
    };
    let team = ok(env
        .call(
            &env.owner,
            "POST",
            &format!("/v1/workspaces/{}/teams", env.ws),
            Some(json!({"name": "Dev", "members": [{"userId": env.owner.id, "role": "MANAGER"}]})),
        )
        .await)["id"]
        .as_str()
        .unwrap()
        .to_string();
    ok(env
        .call(
            &env.owner,
            "PUT",
            &format!(
                "/v1/workspaces/{}/teams/{team}/members/{}",
                env.ws, by_team.id
            ),
            Some(json!({"role": "MEMBER"})),
        )
        .await);
    let project = ok(env
        .call(
            &env.owner,
            "POST",
            &format!("/v1/workspaces/{}/projects", env.ws),
            Some(json!({"name": "Alpha", "teamIds": [team]})),
        )
        .await)["id"]
        .as_str()
        .unwrap()
        .to_string();
    ok(env
        .call(
            &env.owner,
            "PUT",
            &format!("/v1/projects/{project}/teams/{team}/participation"),
            Some(json!({"role": "EDITOR"})),
        )
        .await);
    ok(env
        .call(
            &env.owner,
            "PUT",
            &format!("/v1/projects/{project}/grants/{}", direct.id),
            Some(json!({"role": "EDITOR"})),
        )
        .await);

    let tickets = format!("/v1/projects/{project}/tickets");
    let reaches = |who: &common::Person| {
        let (env, who, path) = (&env, who.clone(), tickets.clone());
        async move { env.call(&who, "GET", &path, None).await.status }
    };
    let rows = |pool: PgPool, user: Uuid| async move {
        sqlx::query_as::<_, (i64, i64)>(
            "SELECT (SELECT count(*) FROM penfii.team_members WHERE user_id = $1),
                    (SELECT count(*) FROM penfii.project_user_grants WHERE user_id = $1)",
        )
        .bind(user)
        .fetch_one(&pool)
        .await
        .unwrap()
    };
    for who in [&by_team, &direct] {
        assert_eq!(reaches(who).await, StatusCode::OK, "before the removal");
    }

    for who in [&by_team, &direct] {
        penfii_core::admin::remove_member(&pool, env.owner.id, env.ws, who.id)
            .await
            .unwrap();
    }
    // Removed: the Workspace is closed to them, so neither the Team nor the grant reaches anything.
    for who in [&by_team, &direct] {
        assert_eq!(reaches(who).await, StatusCode::NOT_FOUND, "while removed");
    }
    // What the removal leaves behind is dormant, not deleted.
    assert_eq!(rows(pool.clone(), by_team.id).await, (1, 0));
    assert_eq!(rows(pool.clone(), direct.id).await, (0, 1));

    // Back, with a Workspace role of the operator's choosing: the organisation they belonged to is theirs again.
    for who in [&by_team, &direct] {
        assert_eq!(
            join_workspace_as_operator(&pool, env.ws, who.id, "MEMBER")
                .await
                .unwrap(),
            JoinOutcome::Rejoined
        );
    }
    for who in [&by_team, &direct] {
        assert_eq!(reaches(who).await, StatusCode::OK, "after the rejoin");
    }
    assert_eq!(rows(pool.clone(), by_team.id).await, (1, 0));
    assert_eq!(rows(pool.clone(), direct.id).await, (0, 1));
    let direct_role: String =
        sqlx::query_scalar("SELECT role FROM penfii.project_user_grants WHERE user_id = $1")
            .bind(direct.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(direct_role, "EDITOR");
    let team_role: String =
        sqlx::query_scalar("SELECT role FROM penfii.team_members WHERE user_id = $1")
            .bind(by_team.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(team_role, "MEMBER");
}

/// Disabling an account is not leaving the Workspace: the membership, and with it the Team and
/// Project access, is untouched and needs no rejoin when the account is enabled again.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn enabling_an_account_needs_no_rejoin_and_changes_no_membership(pool: PgPool) {
    let env = common::Env::new(pool.clone()).await;
    let person = env.person("paused", "ADMIN").await;
    penfii_core::admin::operator_set_account_disabled(&pool, &person.email, true)
        .await
        .unwrap();
    assert_eq!(
        membership(&pool, env.ws, person.id).await,
        Some(("ADMIN".into(), "ACTIVE".into(), false))
    );
    penfii_core::admin::operator_set_account_disabled(&pool, &person.email, false)
        .await
        .unwrap();
    assert_eq!(
        membership(&pool, env.ws, person.id).await,
        Some(("ADMIN".into(), "ACTIVE".into(), false))
    );
    // Nothing was joined again.
    assert_eq!(joined_entries(&pool, env.ws, person.id).await.len(), 1);
    assert!(matches!(
        join_workspace_as_operator(&pool, env.ws, person.id, "MEMBER")
            .await
            .unwrap(),
        JoinOutcome::AlreadyMember { .. }
    ));
}
