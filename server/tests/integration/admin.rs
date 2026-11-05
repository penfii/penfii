//! Workspace administration: members and roles, account state, invitations, team membership, the
//! audit log, Ticket trash and purge, and the health probes.

use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::common::{
    self, MEMBER_PASSWORD as PASSWORD, Person, call_parts as call, doc, limits, login,
};

/// The shared fixture, with the shapes this file reads: `(status, body)` pairs.
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
    async fn invite(&self, by: &Person, email: &str, role: &str) -> (StatusCode, Value) {
        let (status, body, _) = call(
            &self.app,
            "POST",
            &format!("/v1/workspaces/{}/invitations", self.ws),
            Some(&by.cookie),
            Some(json!({"email": email, "role": role})),
        )
        .await;
        (status, body)
    }

    fn url(&self, tail: &str) -> String {
        format!("/v1/workspaces/{}/{tail}", self.ws)
    }

    async fn get(&self, who: &Person, path: &str) -> (StatusCode, Value) {
        let (status, body, _) = call(&self.app, "GET", path, Some(&who.cookie), None).await;
        (status, body)
    }

    async fn send(
        &self,
        who: &Person,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let (status, body, _) = call(&self.app, method, path, Some(&who.cookie), body).await;
        (status, body)
    }

    async fn members(&self, who: &Person) -> Vec<Value> {
        let (status, body) = self.get(who, &self.url("members")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["items"].as_array().unwrap().clone()
    }

    async fn audit_actions(&self) -> Vec<String> {
        let (status, body) = self.get(&self.owner, &self.url("audit?limit=200")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["action"].as_str().unwrap().to_string())
            .collect()
    }

    async fn age_sessions(&self) {
        sqlx::query("UPDATE penfii.sessions SET last_reauth_at = now() - interval '2 hours'")
            .execute(&self.pool)
            .await
            .unwrap();
    }
}

fn member_of(items: &[Value], id: Uuid) -> &Value {
    items
        .iter()
        .find(|item| item["userId"] == json!(id))
        .expect("member listed")
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn member_list_and_role_changes_respect_roles_and_workspaces(pool: PgPool) {
    let env = setup(pool).await;
    let admin = env.person("admin", "ADMIN").await;
    let member = env.person("member", "MEMBER").await;
    let other = env.person("other", "MEMBER").await;
    let outsider = env.outsider().await;

    // Who may list: owner and admin yes, a plain member no, a stranger sees nothing.
    assert_eq!(env.members(&env.owner).await.len(), 4);
    assert_eq!(env.members(&admin).await.len(), 4);
    assert_eq!(
        env.get(&member, &env.url("members")).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        env.get(&outsider, &env.url("members")).await.0,
        StatusCode::NOT_FOUND
    );
    let (status, _, _) = call(&env.app, "GET", &env.url("members"), None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Role changes are the owner's alone.
    let path = env.url(&format!("members/{}", member.id));
    for who in [&member, &admin] {
        let (status, body) = env
            .send(who, "PATCH", &path, Some(json!({"role": "ADMIN"})))
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    }
    let (status, _) = env
        .send(&outsider, "PATCH", &path, Some(json!({"role": "ADMIN"})))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = env
        .send(&env.owner, "PATCH", &path, Some(json!({"role": "BOSS"})))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, body) = env
        .send(
            &env.owner,
            "PATCH",
            &env.url(&format!("members/{}", env.owner.id)),
            Some(json!({"role": "MEMBER"})),
        )
        .await;
    assert_eq!(
        (status, body["code"].as_str()),
        (StatusCode::CONFLICT, Some("cannot_modify_self"))
    );
    let (status, body) = env
        .send(&env.owner, "PATCH", &path, Some(json!({"role": "ADMIN"})))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert_eq!(
        member_of(&env.members(&env.owner).await, member.id)["role"],
        "ADMIN"
    );
    // The same role again changes nothing and is not logged twice.
    let (status, _) = env
        .send(&env.owner, "PATCH", &path, Some(json!({"role": "ADMIN"})))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let changes = env
        .audit_actions()
        .await
        .iter()
        .filter(|a| *a == "member.role_changed")
        .count();
    assert_eq!(changes, 1);
    // A person who is not an active member is not found.
    let (status, _) = env
        .send(
            &env.owner,
            "PATCH",
            &env.url(&format!("members/{}", outsider.id)),
            Some(json!({"role": "ADMIN"})),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // The new admin now manages plain members, but not the owner, not themself.
    let (status, _) = env
        .send(
            &member,
            "DELETE",
            &env.url(&format!("members/{}", env.owner.id)),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, body) = env.send(&member, "DELETE", &path, None).await;
    assert_eq!(
        (status, body["code"].as_str()),
        (StatusCode::CONFLICT, Some("cannot_modify_self"))
    );
    let (status, _) = env
        .send(
            &member,
            "DELETE",
            &env.url(&format!("members/{}", admin.id)),
            None,
        )
        .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "an admin does not remove another admin"
    );
    let (status, _) = env
        .send(
            &other,
            "DELETE",
            &env.url(&format!("members/{}", member.id)),
            None,
        )
        .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a plain member cannot remove anyone"
    );
    let (status, _) = env
        .send(
            &outsider,
            "DELETE",
            &env.url(&format!("members/{}", other.id)),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // Another Workspace's admin has no hold on this one even with a guessed id.
    let (status, _) = env
        .send(
            &admin,
            "DELETE",
            &env.url(&format!("members/{}", outsider.id)),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn removed_member_loses_access_at_once_and_returns_by_invitation(pool: PgPool) {
    let env = setup(pool).await;
    let admin = env.person("admin", "ADMIN").await;
    let member = env.person("member", "MEMBER").await;

    // Things the member could reach: a public channel, a DM with the owner.
    let (status, channel, _) = call(
        &env.app,
        "POST",
        &env.url("channels"),
        Some(&env.owner.cookie),
        Some(json!({"name": "town", "visibility": "PUBLIC", "teamId": null, "projectId": null})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{channel}");
    let channel_id = channel["id"].as_str().unwrap().to_string();
    let (status, dm, _) = call(
        &env.app,
        "POST",
        &env.url("dms"),
        Some(&env.owner.cookie),
        Some(json!({"userIds": [env.owner.id, member.id]})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{dm}");
    let dm_id = dm["id"].as_str().unwrap().to_string();
    let message = json!({"bodySchemaVersion": 1, "body": doc("hello"), "threadRootId": null, "replyToId": null});
    let (status, body) = env
        .send(
            &member,
            "POST",
            &format!("/v1/conversations/{dm_id}/messages"),
            Some(message.clone()),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(
        env.get(&member, &format!("/v1/conversations/{channel_id}"))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(env.get(&member, &env.url("space")).await.0, StatusCode::OK);

    // Removing needs a recent sign-in.
    env.age_sessions().await;
    let path = env.url(&format!("members/{}", member.id));
    let (status, body) = env.send(&admin, "DELETE", &path, None).await;
    assert_eq!(
        (status, body["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("reauth_required"))
    );
    let (status, cookie) = login(&env.app, "admin@penfii.test", PASSWORD).await;
    assert_eq!(status, StatusCode::OK);
    let admin = Person {
        cookie: cookie.unwrap(),
        ..admin
    };
    let (status, body) = env.send(&admin, "DELETE", &path, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    // The old session is still a valid sign-in, but the Workspace is closed to it.
    assert_eq!(env.get(&member, "/v1/me").await.0, StatusCode::OK);
    assert_eq!(
        env.get(&member, &env.url("space")).await.0,
        StatusCode::NOT_FOUND
    );
    let (_, workspaces) = env.get(&member, "/v1/workspaces").await;
    assert_eq!(workspaces.as_array().map(Vec::len), Some(0), "{workspaces}");
    for conversation in [&channel_id, &dm_id] {
        let base = format!("/v1/conversations/{conversation}");
        assert_eq!(
            env.get(&member, &base).await.0,
            StatusCode::NOT_FOUND,
            "read {base}"
        );
        assert_eq!(
            env.get(&member, &format!("{base}/changes")).await.0,
            StatusCode::NOT_FOUND
        );
        let (status, _) = env
            .send(
                &member,
                "POST",
                &format!("{base}/messages"),
                Some(message.clone()),
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "write {base}");
    }
    let (status, _) = env
        .send(
            &member,
            "POST",
            &env.url("channels"),
            Some(
                json!({"name": "late", "visibility": "PUBLIC", "teamId": null, "projectId": null}),
            ),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // Listed as removed, with the time.
    let listed = env.members(&env.owner).await;
    let row = member_of(&listed, member.id);
    assert_eq!(row["state"], "INACTIVE");
    assert!(row["deactivatedAt"].is_string());
    assert_eq!(row["accountDisabled"], false);
    // Removing again is "not found", not a second audit entry.
    let (status, _) = env.send(&admin, "DELETE", &path, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        env.audit_actions()
            .await
            .iter()
            .filter(|a| *a == "member.removed")
            .count(),
        1
    );

    // An invitation brings the person back with the invited role and their old history.
    let (status, invitation) = env.invite(&env.owner, &member.email, "ADMIN").await;
    assert_eq!(status, StatusCode::CREATED, "{invitation}");
    let (status, body, _) = call(
        &env.app,
        "POST",
        "/v1/invitations/accept",
        None,
        Some(json!({"token": invitation["inviteToken"], "displayName": "member", "password": PASSWORD})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let listed = env.members(&env.owner).await;
    let row = member_of(&listed, member.id);
    assert_eq!(
        (row["state"].as_str(), row["role"].as_str()),
        (Some("ACTIVE"), Some("ADMIN"))
    );
    assert_eq!(
        env.get(&member, &format!("/v1/conversations/{dm_id}"))
            .await
            .0,
        StatusCode::OK
    );
    let actions = env.audit_actions().await;
    assert!(
        actions.contains(&"member.joined".to_string()),
        "{actions:?}"
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn disabling_an_account_ends_every_session_and_blocks_sign_in(pool: PgPool) {
    let env = setup(pool).await;
    let admin = env.person("admin", "ADMIN").await;
    let member = env.person("member", "MEMBER").await;
    let second_session = {
        let (status, cookie) = login(&env.app, &member.email, PASSWORD).await;
        assert_eq!(status, StatusCode::OK);
        cookie.unwrap()
    };
    let outsider = env.outsider().await;
    let path = env.url(&format!("members/{}/account", member.id));

    // Not for admins, plain members or strangers; not on yourself.
    for (who, expected) in [
        (&admin, StatusCode::FORBIDDEN),
        (&member, StatusCode::FORBIDDEN),
        (&outsider, StatusCode::NOT_FOUND),
    ] {
        let (status, body) = env
            .send(who, "PUT", &path, Some(json!({"disabled": true})))
            .await;
        assert_eq!(status, expected, "{body}");
    }
    let (status, body) = env
        .send(
            &env.owner,
            "PUT",
            &env.url(&format!("members/{}/account", env.owner.id)),
            Some(json!({"disabled": true})),
        )
        .await;
    assert_eq!(
        (status, body["code"].as_str()),
        (StatusCode::CONFLICT, Some("cannot_modify_self"))
    );
    assert_eq!(
        env.get(&member, "/v1/me").await.0,
        StatusCode::OK,
        "nothing changed yet"
    );
    // A person outside this Workspace cannot be disabled through it.
    let (status, _) = env
        .send(
            &env.owner,
            "PUT",
            &env.url(&format!("members/{}/account", outsider.id)),
            Some(json!({"disabled": true})),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // An account that is active in another Workspace is protected from this instance-wide switch.
    sqlx::query("INSERT INTO penfii.workspace_members (workspace_id, user_id, role, state) SELECT w.id, $1, 'MEMBER', 'ACTIVE' FROM penfii.workspaces w WHERE w.name = 'Other'")
        .bind(member.id)
        .execute(&env.pool)
        .await
        .unwrap();
    let (status, body) = env
        .send(&env.owner, "PUT", &path, Some(json!({"disabled": true})))
        .await;
    assert_eq!(
        (status, body["code"].as_str()),
        (StatusCode::CONFLICT, Some("account_in_other_workspace"))
    );
    sqlx::query("DELETE FROM penfii.workspace_members WHERE user_id = $1 AND workspace_id <> $2")
        .bind(member.id)
        .bind(env.ws)
        .execute(&env.pool)
        .await
        .unwrap();

    // Disabling needs a recent sign-in, then ends every session at once.
    env.age_sessions().await;
    let (status, body) = env
        .send(&env.owner, "PUT", &path, Some(json!({"disabled": true})))
        .await;
    assert_eq!(
        (status, body["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("reauth_required"))
    );
    let (status, cookie) = login(&env.app, &env.owner.email, "owner-test-passphrase").await;
    assert_eq!(status, StatusCode::OK);
    let owner = Person {
        cookie: cookie.unwrap(),
        ..env.owner.clone()
    };
    let (status, body) = env
        .send(&owner, "PUT", &path, Some(json!({"disabled": true})))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    for cookie in [&member.cookie, &second_session] {
        let (status, _, _) = call(&env.app, "GET", "/v1/me", Some(cookie), None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let (status, _, _) = call(&env.app, "GET", &env.url("space"), Some(cookie), None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    assert_eq!(
        login(&env.app, &member.email, PASSWORD).await.0,
        StatusCode::UNAUTHORIZED
    );
    let live: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.sessions WHERE user_id = $1 AND revoked_at IS NULL",
    )
    .bind(member.id)
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(live, 0);
    assert_eq!(
        member_of(&env.members(&owner).await, member.id)["accountDisabled"],
        true
    );
    // A disabled account cannot be brought back in through an invitation either.
    let (status, invitation) = env.invite(&owner, &member.email, "MEMBER").await;
    assert_eq!(status, StatusCode::CONFLICT, "{invitation}");
    assert_eq!(invitation["code"], "already_member");

    // Enabling lets the person sign in again; the membership was never touched.
    let (status, _) = env
        .send(&owner, "PUT", &path, Some(json!({"disabled": false})))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, cookie) = login(&env.app, &member.email, PASSWORD).await;
    assert_eq!(status, StatusCode::OK);
    let back = Person {
        cookie: cookie.unwrap(),
        ..member.clone()
    };
    assert_eq!(env.get(&back, &env.url("space")).await.0, StatusCode::OK);
    let actions = env.audit_actions().await;
    assert!(
        actions.contains(&"user.disabled".to_string())
            && actions.contains(&"user.enabled".to_string()),
        "{actions:?}"
    );

    // The operator's switch (admin-cli) does the same when no owner can act.
    penfii_core::admin::operator_set_account_disabled(&env.pool, &member.email, true)
        .await
        .unwrap();
    assert_eq!(env.get(&back, "/v1/me").await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(
        login(&env.app, &member.email, PASSWORD).await.0,
        StatusCode::UNAUTHORIZED
    );
    penfii_core::admin::operator_set_account_disabled(&env.pool, &member.email, false)
        .await
        .unwrap();
    assert_eq!(
        login(&env.app, &member.email, PASSWORD).await.0,
        StatusCode::OK
    );
    assert!(
        penfii_core::admin::operator_set_account_disabled(&env.pool, "nobody@penfii.test", true)
            .await
            .is_err()
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn invitations_are_listed_with_a_status_and_a_revoked_link_stops_working(pool: PgPool) {
    let env = setup(pool).await;
    let admin = env.person("admin", "ADMIN").await;
    let member = env.person("member", "MEMBER").await;
    let outsider = env.outsider().await;

    let (_, pending) = env
        .invite(&env.owner, "pending@penfii.test", "MEMBER")
        .await;
    let (_, revoked) = env
        .invite(&env.owner, "revoked@penfii.test", "MEMBER")
        .await;
    let (_, expired) = env
        .invite(&env.owner, "expired@penfii.test", "MEMBER")
        .await;
    let (_, admin_invite) = env
        .invite(&env.owner, "future-admin@penfii.test", "ADMIN")
        .await;
    sqlx::query("UPDATE penfii.workspace_invitations SET expires_at = now() - interval '1 minute' WHERE email_key = 'expired@penfii.test'")
        .execute(&env.pool)
        .await
        .unwrap();

    // The list is for managers only.
    let (status, _) = env.get(&member, &env.url("invitations")).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = env.get(&outsider, &env.url("invitations")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let revoke_path = |invitation: &Value| {
        env.url(&format!(
            "invitations/{}",
            invitation["id"].as_str().unwrap()
        ))
    };
    // A plain member and a stranger cannot revoke; an admin cannot revoke an admin invitation.
    assert_eq!(
        env.send(&member, "DELETE", &revoke_path(&pending), None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        env.send(&outsider, "DELETE", &revoke_path(&pending), None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        env.send(&admin, "DELETE", &revoke_path(&admin_invite), None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        env.send(
            &admin,
            "DELETE",
            &env.url(&format!("invitations/{}", Uuid::new_v4())),
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    // An admin invites plain members only.
    let (status, _) = env.invite(&admin, "x-owner@penfii.test", "OWNER").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = env.invite(&admin, "x-admin@penfii.test", "ADMIN").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = env.invite(&admin, "x-member@penfii.test", "MEMBER").await;
    assert_eq!(status, StatusCode::CREATED);
    // Someone who is already a member cannot be invited again.
    let (status, body) = env.invite(&env.owner, &member.email, "MEMBER").await;
    assert_eq!(
        (status, body["code"].as_str()),
        (StatusCode::CONFLICT, Some("already_member"))
    );

    // Revoke: the link no longer previews or accepts.
    let token = revoked["inviteToken"].clone();
    async fn preview(app: &axum::Router, token: Value) -> StatusCode {
        call(
            app,
            "POST",
            "/v1/invitations/preview",
            None,
            Some(json!({"token": token})),
        )
        .await
        .0
    }
    assert_eq!(preview(&env.app, token.clone()).await, StatusCode::OK);
    assert_eq!(
        env.send(&admin, "DELETE", &revoke_path(&revoked), None)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        preview(&env.app, token.clone()).await,
        StatusCode::NOT_FOUND
    );
    let (status, _, _) = call(
        &env.app,
        "POST",
        "/v1/invitations/accept",
        None,
        Some(json!({"token": token, "displayName": "R", "password": PASSWORD})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let no_user: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.users WHERE email_key = 'revoked@penfii.test'",
    )
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(no_user, 0);

    // Accepted, expired and already revoked ones cannot be revoked; the statuses are told apart.
    let (status, body) = env
        .send(&env.owner, "DELETE", &revoke_path(&revoked), None)
        .await;
    assert_eq!(
        (status, body["code"].as_str()),
        (StatusCode::CONFLICT, Some("invitation_not_pending"))
    );
    assert_eq!(
        env.send(&env.owner, "DELETE", &revoke_path(&expired), None)
            .await
            .0,
        StatusCode::CONFLICT
    );
    let (_, listed) = env.get(&env.owner, &env.url("invitations")).await;
    let status_of = |email: &str| {
        listed["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["email"] == email)
            .map(|item| item["status"].as_str().unwrap().to_string())
    };
    assert_eq!(status_of("pending@penfii.test").as_deref(), Some("PENDING"));
    assert_eq!(status_of("revoked@penfii.test").as_deref(), Some("REVOKED"));
    assert_eq!(status_of("expired@penfii.test").as_deref(), Some("EXPIRED"));
    assert_eq!(status_of("member@penfii.test").as_deref(), Some("ACCEPTED"));
    assert!(
        !listed.to_string().contains("inviteToken")
            && !listed
                .to_string()
                .contains(pending["inviteToken"].as_str().unwrap())
    );
    let (status, _) = env
        .send(&env.owner, "DELETE", &revoke_path(&admin_invite), None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "an owner revokes any");
    let actions = env.audit_actions().await;
    assert!(
        actions.contains(&"invitation.created".to_string())
            && actions.contains(&"invitation.revoked".to_string()),
        "{actions:?}"
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn teams_are_created_by_owners_and_admins_and_managed_with_team_managers(pool: PgPool) {
    let env = setup(pool).await;
    let admin = env.person("admin", "ADMIN").await;
    let member = env.person("member", "MEMBER").await;
    let lead = env.person("lead", "MEMBER").await;
    let outsider = env.outsider().await;
    let teams = env.url("teams");
    let create = |who: &Person, body: Value| {
        let env = &env;
        let teams = teams.clone();
        let cookie = who.cookie.clone();
        async move { call(&env.app, "POST", &teams, Some(&cookie), Some(body)).await }
    };

    // Creating a Team is for owners and admins, needs at least one member with an explicit role, and does
    // not put the creator in the Team.
    let first = json!({"userId": lead.id, "role": "MANAGER"});
    assert_eq!(
        create(&member, json!({"name": "Core", "members": [first]}))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        create(&admin, json!({"name": "Core"})).await.0,
        StatusCode::BAD_REQUEST
    );
    for bad in [
        json!({"name": "Core", "members": []}),
        json!({"name": "Core", "members": [{"userId": lead.id, "role": "MEMBER"}]}),
        json!({"name": "Core", "members": [{"userId": lead.id, "role": "OWNER"}]}),
        json!({"name": "Core", "members": [{"userId": outsider.id, "role": "MEMBER"}]}),
        json!({"name": "Core", "members": [first, {"userId": lead.id, "role": "MEMBER"}]}),
    ] {
        let (status, body, _) = create(&admin, bad).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    }
    let (status, team, _) = create(&admin, json!({"name": "Core", "members": [first]})).await;
    assert_eq!(status, StatusCode::CREATED, "{team}");
    let team_id = team["id"].as_str().unwrap().to_string();
    let list = env.url(&format!("teams/{team_id}/members"));
    let entry = |who: Uuid| env.url(&format!("teams/{team_id}/members/{who}"));
    let put = |who: &Person, target: Uuid, role: &str| {
        let (env, url, who) = (&env, entry(target), who.clone());
        let role = role.to_owned();
        async move {
            env.send(&who, "PUT", &url, Some(json!({"role": role})))
                .await
                .0
        }
    };
    let (_, members) = env.get(&admin, &list).await;
    let roles: Vec<(&str, &str)> = members["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| (m["userId"].as_str().unwrap(), m["role"].as_str().unwrap()))
        .collect();
    assert_eq!(
        roles,
        vec![(lead.id.to_string().as_str(), "MANAGER")],
        "{members}"
    );

    // The only manager can neither be demoted nor removed (by an admin or by themselves) until there is another.
    for who in [&admin, &lead] {
        assert_eq!(put(who, lead.id, "MEMBER").await, StatusCode::CONFLICT);
        let (status, body) = env.send(who, "DELETE", &entry(lead.id), None).await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(body["code"], "last_team_manager");
    }
    // Every workspace member can read the roster. Changing it still takes a manager.
    assert_eq!(env.get(&member, &list).await.0, StatusCode::OK);
    assert_eq!(env.get(&outsider, &list).await.0, StatusCode::NOT_FOUND);
    assert_eq!(
        put(&member, member.id, "MEMBER").await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        put(&outsider, member.id, "MEMBER").await,
        StatusCode::NOT_FOUND
    );
    // Only active members of the Workspace can join a Team, with a valid role.
    assert_eq!(
        put(&admin, outsider.id, "MEMBER").await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        put(&admin, member.id, "BOSS").await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        put(&admin, member.id, "MEMBER").await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        put(&admin, member.id, "MEMBER").await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        member_of(&env.members(&admin).await, member.id)["teams"][0]["name"],
        "Core"
    );
    // A plain Team member still manages nothing; the Team's manager manages members and roles.
    assert_eq!(
        put(&member, admin.id, "MEMBER").await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        env.send(&member, "DELETE", &entry(lead.id), None).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(env.get(&lead, &list).await.0, StatusCode::OK);
    assert_eq!(
        put(&lead, member.id, "MANAGER").await,
        StatusCode::NO_CONTENT
    );
    let (_, members) = env.get(&lead, &list).await;
    assert!(
        members["items"].to_string().contains("MANAGER")
            && members["items"].as_array().unwrap().len() == 2,
        "{members}"
    );
    // With a second manager the first can step down, and the second is then the last one.
    assert_eq!(put(&admin, lead.id, "MEMBER").await, StatusCode::NO_CONTENT);
    assert_eq!(
        env.send(&admin, "DELETE", &entry(member.id), None).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        put(&admin, lead.id, "MANAGER").await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        env.send(&lead, "DELETE", &entry(member.id), None).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        env.send(&admin, "DELETE", &entry(member.id), None).await.0,
        StatusCode::NO_CONTENT
    );
    let (_, members) = env.get(&admin, &list).await;
    assert!(!members.to_string().contains(&member.id.to_string()));
    // The manager of one Team does not manage another. They can still read its roster.
    // A Team of another Workspace is not reachable.
    let (status, other, _) = create(
        &admin,
        json!({"name": "Other", "members": [{"userId": member.id, "role": "MANAGER"}]}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{other}");
    let other_list = env.url(&format!("teams/{}/members", other["id"].as_str().unwrap()));
    assert_eq!(env.get(&lead, &other_list).await.0, StatusCode::OK);
    assert_eq!(
        env.get(
            &admin,
            &env.url(&format!("teams/{}/members", Uuid::new_v4()))
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let logged = env
        .audit_actions()
        .await
        .iter()
        .filter(|a| a.starts_with("team.member_"))
        .count();
    assert_eq!(
        logged, 5,
        "adds, role changes and removals are logged; repeats are not"
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn audit_log_is_for_managers_and_auditors_of_that_workspace_only(pool: PgPool) {
    let env = setup(pool).await;
    let admin = env.person("admin", "ADMIN").await;
    let member = env.person("member", "MEMBER").await;
    let auditor = env.person("auditor", "MEMBER").await;
    let outsider = env.outsider().await;
    let (status, _) = env
        .send(
            &env.owner,
            "PATCH",
            &env.url(&format!("members/{}", member.id)),
            Some(json!({"role": "ADMIN"})),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    // The other Workspace has its own history.
    let (status, _) = env.invite(&outsider, "someone@other.test", "MEMBER").await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "outsider is not a manager of W"
    );
    let other_ws: Uuid =
        sqlx::query_scalar("SELECT id FROM penfii.workspaces WHERE name = 'Other'")
            .fetch_one(&env.pool)
            .await
            .unwrap();
    let (status, _, _) = call(
        &env.app,
        "POST",
        &format!("/v1/workspaces/{other_ws}/invitations"),
        Some(&outsider.cookie),
        Some(json!({"email": "someone@other.test", "role": "MEMBER"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, page) = env.get(&env.owner, &env.url("audit")).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    let text = page.to_string();
    assert!(
        !text.contains("someone@other.test"),
        "another Workspace's entries leaked: {text}"
    );
    assert!(!text.contains("inviteToken"));
    assert_eq!(env.get(&admin, &env.url("audit")).await.0, StatusCode::OK);
    assert_eq!(
        env.get(&auditor, &env.url("audit")).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        env.get(&outsider, &env.url("audit")).await.0,
        StatusCode::NOT_FOUND
    );
    sqlx::query(
        "INSERT INTO penfii.workspace_security_auditors (workspace_id, user_id) VALUES ($1, $2)",
    )
    .bind(env.ws)
    .bind(auditor.id)
    .execute(&env.pool)
    .await
    .unwrap();
    assert_eq!(env.get(&auditor, &env.url("audit")).await.0, StatusCode::OK);

    // Filter by prefix, and page through newest first.
    let (_, only) = env.get(&env.owner, &env.url("audit?action=member.")).await;
    assert!(
        only["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["action"].as_str().unwrap().starts_with("member."))
    );
    assert!(!only["items"].as_array().unwrap().is_empty());
    let (_, first) = env.get(&env.owner, &env.url("audit?limit=2")).await;
    assert_eq!(first["items"].as_array().unwrap().len(), 2);
    let cursor = first["nextCursor"]
        .as_str()
        .expect("more entries")
        .to_string();
    let (_, second) = env
        .get(
            &env.owner,
            &env.url(&format!(
                "audit?limit=200&cursor={}",
                cursor.replace('+', "%2B").replace('|', "%7C")
            )),
        )
        .await;
    let seen: std::collections::HashSet<String> = first["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].to_string())
        .collect();
    assert!(
        second["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| !seen.contains(&i["id"].to_string()))
    );
    let (status, _) = env.get(&env.owner, &env.url("audit?cursor=nonsense")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // The actor and the change are recorded.
    let (_, changed) = env
        .get(&env.owner, &env.url("audit?action=member.role_changed"))
        .await;
    let entry = &changed["items"][0];
    assert_eq!(entry["actorUserId"], json!(env.owner.id));
    assert_eq!(entry["data"]["after"], "ADMIN");
    // The entry names its target, so the log says who it was about.
    assert_eq!(entry["targetLabel"], "member");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn trash_restore_and_purge_a_ticket(pool: PgPool) {
    let env = setup(pool).await;
    let admin = env.person("admin", "ADMIN").await;
    let editor = env.person("editor", "MEMBER").await;
    let outsider = env.outsider().await;

    // `editor` becomes a Project editor through a Team linked to the Project.
    let (_, team, _) = call(
        &env.app,
        "POST",
        &env.url("teams"),
        Some(&env.owner.cookie),
        Some(json!({"name": "Dev", "members": [{"userId": editor.id, "role": "MANAGER"}]})),
    )
    .await;
    let team_id = team["id"].as_str().unwrap().to_string();
    let (status, _) = env
        .send(
            &env.owner,
            "PUT",
            &env.url(&format!("teams/{team_id}/members/{}", editor.id)),
            Some(json!({"role": "MANAGER"})),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, project, _) = call(
        &env.app,
        "POST",
        &env.url("projects"),
        Some(&env.owner.cookie),
        Some(json!({"name": "Trash", "teamIds": [team_id]})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    let project_id = project["id"].as_str().unwrap().to_string();
    let (status, _) = env
        .send(
            &env.owner,
            "PUT",
            &format!("/v1/projects/{project_id}/teams/{team_id}/participation"),
            Some(json!({"role": "EDITOR"})),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let create = |body: Value| {
        let env = &env;
        let project_id = project_id.clone();
        async move {
            let (status, ticket) = env
                .send(
                    &env.owner,
                    "POST",
                    &format!("/v1/projects/{project_id}/tickets"),
                    Some(body),
                )
                .await;
            assert_eq!(status, StatusCode::CREATED, "{ticket}");
            ticket["id"].as_str().unwrap().parse::<Uuid>().unwrap()
        }
    };
    let parent = create(
        json!({"title": "secret plan", "hierarchyKind": "TASK", "description": doc("do not keep")}),
    )
    .await;
    let child = {
        create(json!({"title": "secret step", "hierarchyKind": "SUBTASK", "parentId": parent}))
            .await
    };
    let (status, comment) = env
        .send(
            &env.owner,
            "POST",
            &format!("/v1/tickets/{parent}/comments"),
            Some(json!({"body": doc("private remark")})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{comment}");

    let cmd = |ticket: Uuid, name: &str| format!("/v1/tickets/{ticket}/commands/{name}");
    // Trash belongs to the Project admin; an editor, a stranger and the anonymous caller are refused.
    assert_eq!(
        env.send(&editor, "POST", &cmd(parent, "trash"), None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        env.send(&outsider, "POST", &cmd(parent, "trash"), None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        env.get(&editor, &format!("/v1/projects/{project_id}/trash"))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        env.get(&outsider, &format!("/v1/projects/{project_id}/trash"))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let (status, _, _) = call(&env.app, "POST", &cmd(parent, "trash"), None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    assert_eq!(
        env.send(&env.owner, "POST", &cmd(parent, "trash"), None)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    let (status, body) = env
        .send(&env.owner, "POST", &cmd(parent, "trash"), None)
        .await;
    assert_eq!(
        (status, body["code"].as_str()),
        (StatusCode::CONFLICT, Some("already_trashed"))
    );
    // Out of sight for everyone, listed for the admin.
    assert_eq!(
        env.get(&env.owner, &format!("/v1/tickets/{parent}"))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let (_, board) = env
        .get(&env.owner, &format!("/v1/projects/{project_id}/board"))
        .await;
    assert!(!board.to_string().contains("secret plan"), "{board}");
    let (_, trash) = env
        .get(&env.owner, &format!("/v1/projects/{project_id}/trash"))
        .await;
    assert_eq!(trash["items"][0]["title"], "secret plan");
    // Restore brings it back with its content.
    assert_eq!(
        env.send(&editor, "POST", &cmd(parent, "restore-from-trash"), None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        env.send(&env.owner, "POST", &cmd(parent, "restore-from-trash"), None)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        env.get(&env.owner, &format!("/v1/tickets/{parent}"))
            .await
            .1["title"],
        "secret plan"
    );
    let (status, body) = env
        .send(&env.owner, "POST", &cmd(parent, "restore-from-trash"), None)
        .await;
    assert_eq!(
        (status, body["code"].as_str()),
        (StatusCode::CONFLICT, Some("not_trashed"))
    );

    // Purge is a separate, stronger step: only for a trashed Ticket, by the Project admin.
    let (status, body) = env
        .send(&env.owner, "POST", &cmd(parent, "purge"), None)
        .await;
    assert_eq!(
        (status, body["code"].as_str()),
        (StatusCode::CONFLICT, Some("not_trashed"))
    );
    assert_eq!(
        env.send(&env.owner, "POST", &cmd(parent, "trash"), None)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        env.send(&editor, "POST", &cmd(parent, "purge"), None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        env.send(&outsider, "POST", &cmd(parent, "purge"), None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    // Erasing is Project management: a Workspace admin outside the Project is told it does not exist,
    // and one who is only an editor of it is refused. Made the Project's admin, they may.
    assert_eq!(
        env.send(&admin, "POST", &cmd(parent, "purge"), None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    sqlx::query(
        "INSERT INTO penfii.project_user_grants(project_id, user_id, role) VALUES($1, $2, 'EDITOR')",
    )
    .bind(project_id.parse::<Uuid>().unwrap())
    .bind(admin.id)
    .execute(&env.pool)
    .await
    .unwrap();
    assert_eq!(
        env.send(&admin, "POST", &cmd(parent, "purge"), None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    sqlx::query("UPDATE penfii.project_user_grants SET role = 'ADMIN' WHERE project_id = $1 AND user_id = $2")
        .bind(project_id.parse::<Uuid>().unwrap())
        .bind(admin.id)
        .execute(&env.pool)
        .await
        .unwrap();
    // A live child blocks it; so does a dependency.
    let (status, body) = env.send(&admin, "POST", &cmd(parent, "purge"), None).await;
    assert_eq!(
        (status, body["code"].as_str()),
        (StatusCode::CONFLICT, Some("ticket_has_children"))
    );
    assert_eq!(
        env.send(&env.owner, "POST", &cmd(child, "trash"), None)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    let other = create(json!({"title": "other", "hierarchyKind": "TASK"})).await;
    sqlx::query("INSERT INTO penfii.ticket_dependencies (id, workspace_id, project_id, predecessor_id, successor_id, created_by) VALUES ($1, $2, $3, $4, $5, $6)")
        .bind(Uuid::new_v4())
        .bind(env.ws)
        .bind(project_id.parse::<Uuid>().unwrap())
        .bind(other)
        .bind(child)
        .bind(env.owner.id)
        .execute(&env.pool)
        .await
        .unwrap();
    let (status, body) = env.send(&admin, "POST", &cmd(child, "purge"), None).await;
    assert_eq!(
        (status, body["code"].as_str()),
        (StatusCode::CONFLICT, Some("ticket_has_dependencies"))
    );
    sqlx::query("DELETE FROM penfii.ticket_dependencies")
        .execute(&env.pool)
        .await
        .unwrap();

    // Purging needs a recent sign-in.
    env.age_sessions().await;
    let (status, body) = env.send(&admin, "POST", &cmd(child, "purge"), None).await;
    assert_eq!(
        (status, body["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("reauth_required"))
    );
    let (_, cookie) = login(&env.app, &admin.email, PASSWORD).await;
    let admin = Person {
        cookie: cookie.unwrap(),
        ..admin
    };
    assert_eq!(
        env.send(&admin, "POST", &cmd(child, "purge"), None).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        env.send(&admin, "POST", &cmd(parent, "purge"), None)
            .await
            .0,
        StatusCode::NO_CONTENT
    );

    // The content is gone from every table that held it; only a marker stays.
    let row: (String, Value, Option<Uuid>) =
        sqlx::query_as("SELECT title, description, purged_by FROM penfii.tickets WHERE id = $1")
            .bind(parent)
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert_eq!(row.0, "[purged]");
    assert_eq!(row.1, json!({}));
    assert_eq!(row.2, Some(admin.id));
    let leftovers: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM penfii.ticket_revisions WHERE ticket_id = $1 AND snapshot::text LIKE '%secret%')
              + (SELECT count(*) FROM penfii.ticket_comments WHERE ticket_id = $1 AND body::text LIKE '%private remark%')
              + (SELECT count(*) FROM penfii.activity_entries WHERE ticket_id = $1 AND data::text LIKE '%secret%')
              + (SELECT count(*) FROM penfii.domain_events WHERE target_id = $1 AND facts::text LIKE '%secret%')",
    )
    .bind(parent)
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(leftovers, 0);
    assert_eq!(
        env.get(&env.owner, &format!("/v1/tickets/{parent}"))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        env.send(&env.owner, "POST", &cmd(parent, "restore-from-trash"), None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        env.send(&admin, "POST", &cmd(parent, "purge"), None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let (_, trash) = env
        .get(&env.owner, &format!("/v1/projects/{project_id}/trash"))
        .await;
    assert!(trash["items"].as_array().unwrap().is_empty(), "{trash}");
    let actions = env.audit_actions().await;
    for expected in [
        "ticket.trashed",
        "ticket.restored_from_trash",
        "ticket.purged",
    ] {
        assert!(
            actions.contains(&expected.to_string()),
            "{expected} in {actions:?}"
        );
    }
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn health_and_readiness(pool: PgPool) {
    let env = setup(pool).await;
    let (status, body, _) = call(&env.app, "GET", "/healthz", None, None).await;
    assert_eq!(
        (status, body["status"].as_str()),
        (StatusCode::OK, Some("ok"))
    );
    let (status, body, _) = call(&env.app, "GET", "/readyz", None, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "ok");
    assert_eq!(
        body["checks"],
        json!({"database": "ok", "search": "disabled"})
    );

    // Search configured but nobody answers: still serving, reported as degraded.
    let search = penfii_core::search::SearchEngine::new(
        env.pool.clone(),
        penfii_core::search::client::OsClient::new(
            penfii_core::search::client::OsConfig::with_prefix(
                "http://127.0.0.1:9",
                "penfii-health-test",
            ),
        )
        .unwrap(),
    );
    let app = penfii_http_api::router_with(env.pool.clone(), limits(), false, None, Some(search));
    let (status, body, _) = call(&app, "GET", "/readyz", None, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "degraded");
    assert_eq!(body["checks"]["search"], "unavailable");

    // The database gone: liveness holds, readiness fails with 503.
    let broken = sqlx::postgres::PgPoolOptions::new()
        .connect_with(env.pool.connect_options().as_ref().clone())
        .await
        .unwrap();
    let app = penfii_http_api::router(broken.clone(), limits(), false, None);
    broken.close().await;
    let (status, _, _) = call(&app, "GET", "/healthz", None, None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body, _) = call(&app, "GET", "/readyz", None, None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["checks"]["database"], "unavailable");
}

impl Env {
    async fn team(&self, name: &str, manager: &Person) -> String {
        let (status, team, _) = call(
            &self.app,
            "POST",
            &self.url("teams"),
            Some(&self.owner.cookie),
            Some(json!({"name": name, "members": [{"userId": manager.id, "role": "MANAGER"}]})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{team}");
        team["id"].as_str().unwrap().to_string()
    }

    async fn team_state(&self, who: &Person, team_id: &str) -> Value {
        let (status, space) = self.get(who, &self.url("space")).await;
        assert_eq!(status, StatusCode::OK, "{space}");
        space["teams"]
            .as_array()
            .unwrap()
            .iter()
            .find(|team| team["id"] == json!(team_id))
            .cloned()
            .expect("the Team is listed")
    }
}

#[sqlx::test(migrations = false)]
async fn a_team_description_is_plain_text_edited_by_managers_and_owners(pool: PgPool) {
    let env = setup(pool).await;
    let lead = env.person("lead", "MEMBER").await;
    let member = env.person("member", "MEMBER").await;
    let team_id = env.team("Core", &lead).await;
    let path = env.url(&format!("teams/{team_id}"));

    assert_eq!(
        env.team_state(&member, &team_id).await["description"],
        Value::Null
    );
    // A Team manager edits it; a member of the Workspace who does not manage the Team cannot.
    let (status, team) = env
        .send(
            &lead,
            "PATCH",
            &path,
            Some(json!({"description": "  Builds the **core**  "})),
        )
        .await;
    assert_eq!(
        (status, &team["description"]),
        (StatusCode::OK, &json!("Builds the **core**")),
        "{team}"
    );
    // The name stays when only the description is sent.
    assert_eq!(team["name"], "Core");
    assert_eq!(
        env.send(&member, "PATCH", &path, Some(json!({"description": "x"})))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    // There is no product-level limit below the technical guard, and no 280-character cut.
    let long = "가".repeat(1500);
    let (status, team) = env
        .send(
            &env.owner,
            "PATCH",
            &path,
            Some(json!({"description": long})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{team}");
    assert_eq!(team["description"].as_str().unwrap().chars().count(), 1500);
    let (status, body) = env
        .send(
            &env.owner,
            "PATCH",
            &path,
            Some(json!({"description": "a".repeat(2001)})),
        )
        .await;
    assert_eq!(
        (status, body["code"].as_str()),
        (StatusCode::BAD_REQUEST, Some("validation")),
        "{body}"
    );
    // Empty or null clears it; an empty body changes nothing and is refused.
    for clear in [json!({"description": ""}), json!({"description": null})] {
        let (status, team) = env.send(&lead, "PATCH", &path, Some(clear)).await;
        assert_eq!(
            (status, &team["description"]),
            (StatusCode::OK, &Value::Null),
            "{team}"
        );
    }
    assert_eq!(
        env.send(&lead, "PATCH", &path, Some(json!({}))).await.0,
        StatusCode::BAD_REQUEST
    );
    assert!(
        env.audit_actions()
            .await
            .contains(&"team.updated".to_string())
    );
}

#[sqlx::test(migrations = false)]
async fn archiving_a_team_keeps_everything_and_makes_its_channels_read_only_until_restored(
    pool: PgPool,
) {
    let env = setup(pool).await;
    let admin = env.person("admin", "ADMIN").await;
    let lead = env.person("lead", "MEMBER").await;
    let member = env.person("member", "MEMBER").await;
    let newcomer = env.person("newcomer", "MEMBER").await;
    let team_id = env.team("Core", &lead).await;
    let members = env.url(&format!("teams/{team_id}/members/{}", member.id));
    assert_eq!(
        env.send(&env.owner, "PUT", &members, Some(json!({"role": "MEMBER"})))
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    // A Project the Team participates in, and a channel the Team narrows.
    let (status, project, _) = call(
        &env.app,
        "POST",
        &env.url("projects"),
        Some(&env.owner.cookie),
        Some(json!({"name": "Core work", "teamIds": [team_id]})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    let project_id = project["id"].as_str().unwrap().to_string();
    assert_eq!(
        env.send(
            &env.owner,
            "PUT",
            &format!("/v1/projects/{project_id}/teams/{team_id}/participation"),
            Some(json!({"role": "EDITOR"}))
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let (status, channel) = env
        .send(
            &env.owner,
            "POST",
            &env.url("channels"),
            Some(json!({"name": "core-talk", "visibility": "PUBLIC", "teamId": team_id, "projectId": project_id})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{channel}");
    let channel_id = channel["id"].as_str().unwrap().to_string();
    let post = |who: &Person| {
        let (env, path, cookie) = (
            &env,
            format!("/v1/conversations/{channel_id}/messages"),
            who.cookie.clone(),
        );
        async move {
            env.send(
                &Person {
                    id: Uuid::nil(),
                    email: String::new(),
                    cookie,
                },
                "POST",
                &path,
                Some(json!({"bodySchemaVersion": 1, "body": doc("hello")})),
            )
            .await
        }
    };
    let (status, first) = post(&member).await;
    assert_eq!(status, StatusCode::CREATED, "{first}");
    let message_id = first["id"].as_str().unwrap().to_string();

    let archive = env.url(&format!("teams/{team_id}/archive"));
    let restore = env.url(&format!("teams/{team_id}/restore"));
    // Only a Workspace owner or admin archives; a Team manager and a member cannot.
    for who in [&lead, &member] {
        assert_eq!(
            env.send(who, "POST", &archive, None).await.0,
            StatusCode::FORBIDDEN
        );
    }
    let (status, team) = env.send(&admin, "POST", &archive, None).await;
    assert_eq!(status, StatusCode::OK, "{team}");
    assert!(team["archivedAt"].is_string());
    let (status, again) = env.send(&admin, "POST", &archive, None).await;
    assert_eq!(
        (status, again["code"].as_str()),
        (StatusCode::CONFLICT, Some("team_already_archived"))
    );

    // Memberships, roles and Project access are as they were; the roster is still readable.
    let (status, roster) = env
        .get(&member, &env.url(&format!("teams/{team_id}/members")))
        .await;
    assert_eq!(status, StatusCode::OK, "{roster}");
    assert_eq!(roster["items"].as_array().unwrap().len(), 2);
    let (status, projects) = env
        .get(&member, &env.url(&format!("teams/{team_id}/projects")))
        .await;
    assert_eq!(status, StatusCode::OK, "{projects}");
    assert_eq!(projects["items"][0]["teamRole"], "EDITOR");
    assert_eq!(projects["items"][0]["myRole"], "EDITOR");
    assert_eq!(projects["items"][0]["mySources"], json!(["TEAM"]));
    // A member of the Workspace who is not in the Project sees the name and the Team's role, nothing of their own.
    let (_, outside) = env
        .get(&newcomer, &env.url(&format!("teams/{team_id}/projects")))
        .await;
    assert_eq!(outside["items"][0]["canOpen"], false);
    assert_eq!(outside["items"][0]["myRole"], Value::Null);
    assert_eq!(outside["items"][0]["mySources"], json!([]));

    // Writes on the Team stop, in every form.
    let (status, body) = env
        .send(
            &env.owner,
            "PUT",
            &members,
            Some(json!({"role": "MANAGER"})),
        )
        .await;
    assert_eq!(
        (status, body["code"].as_str()),
        (StatusCode::CONFLICT, Some("team_archived")),
        "{body}"
    );
    assert_eq!(
        env.send(&env.owner, "DELETE", &members, None).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        env.send(
            &lead,
            "PATCH",
            &env.url(&format!("teams/{team_id}")),
            Some(json!({"name": "New"}))
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        env.send(
            &env.owner,
            "POST",
            &env.url("channels"),
            Some(json!({"name": "more", "visibility": "PUBLIC", "teamId": team_id, "projectId": project_id}))
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    // The channel is read-only for everyone, and reading still works.
    let (status, body) = post(&member).await;
    assert_eq!(
        (status, body["code"].as_str()),
        (StatusCode::CONFLICT, Some("channel_read_only")),
        "{body}"
    );
    assert_eq!(post(&env.owner).await.0, StatusCode::CONFLICT);
    let edit = common::send(
        &env.app,
        common::request(
            "PATCH",
            &format!("/v1/messages/{message_id}"),
            Some(&member.cookie),
            Some(&Uuid::new_v4().to_string()),
            Some(json!({"body": doc("edited"), "expectedVersion": 1})),
        ),
    )
    .await;
    assert_eq!(edit.status, StatusCode::CONFLICT, "{}", edit.body);
    assert_eq!(
        env.get(&member, &format!("/v1/conversations/{channel_id}/messages"))
            .await
            .0,
        StatusCode::OK
    );
    // The channel row itself is untouched: nothing was written to it.
    let channel_archived: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT archived_at FROM penfii.channels WHERE id = $1")
            .bind(channel_id.parse::<Uuid>().unwrap())
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert!(channel_archived.is_none());

    // Restoring loses nothing: the channel is writable again and the roster is unchanged.
    assert_eq!(
        env.send(&lead, "POST", &restore, None).await.0,
        StatusCode::FORBIDDEN
    );
    let (status, team) = env.send(&admin, "POST", &restore, None).await;
    assert_eq!(
        (status, &team["archivedAt"]),
        (StatusCode::OK, &Value::Null),
        "{team}"
    );
    let (status, again) = env.send(&admin, "POST", &restore, None).await;
    assert_eq!(
        (status, again["code"].as_str()),
        (StatusCode::CONFLICT, Some("team_not_archived"))
    );
    assert_eq!(post(&member).await.0, StatusCode::CREATED);
    let actions = env.audit_actions().await;
    assert!(
        actions.contains(&"team.archived".to_string())
            && actions.contains(&"team.restored".to_string())
    );
}

#[sqlx::test(migrations = false)]
async fn an_invitation_names_a_team_that_is_resolved_when_it_is_accepted(pool: PgPool) {
    let env = setup(pool).await;
    let lead = env.person("lead", "MEMBER").await;
    let team_id = env.team("Core", &lead).await;
    let invite = |email: &str, team: Value| {
        let (env, email) = (&env, email.to_string());
        async move {
            env.send(
                &env.owner,
                "POST",
                &env.url("invitations"),
                Some(json!({"email": email, "role": "MEMBER", "teamId": team})),
            )
            .await
        }
    };
    let accept = |invitation: &Value, name: &str| {
        let (env, token, name) = (&env, invitation["inviteToken"].clone(), name.to_string());
        async move {
            call(
                &env.app,
                "POST",
                "/v1/invitations/accept",
                None,
                Some(json!({"token": token, "displayName": name, "password": PASSWORD})),
            )
            .await
        }
    };
    let role_in_team = |user: &Value| {
        let (pool, team, user) = (
            env.pool.clone(),
            team_id.parse::<Uuid>().unwrap(),
            user["user"]["id"]
                .as_str()
                .unwrap()
                .parse::<Uuid>()
                .unwrap(),
        );
        async move {
            sqlx::query_scalar::<_, String>(
                "SELECT role FROM penfii.team_members WHERE team_id = $1 AND user_id = $2",
            )
            .bind(team)
            .bind(user)
            .fetch_optional(&pool)
            .await
            .unwrap()
        }
    };

    // Only owners and admins invite, a Team manager does not; an unknown Team is not found.
    assert_eq!(
        env.send(
            &lead,
            "POST",
            &env.url("invitations"),
            Some(json!({"email": "m@penfii.test", "role": "MEMBER", "teamId": team_id}))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        invite("none@penfii.test", json!(Uuid::new_v4())).await.0,
        StatusCode::NOT_FOUND
    );

    // An active Team: the person joins the Workspace and the Team as MEMBER.
    let (status, active) = invite("active@penfii.test", json!(team_id)).await;
    assert_eq!(status, StatusCode::CREATED, "{active}");
    let (_, listed) = env.get(&env.owner, &env.url("invitations")).await;
    assert_eq!(listed["items"][0]["teamStatus"], "ACTIVE");
    assert_eq!(listed["items"][0]["teamName"], "Core");
    let (status, joined, _) = accept(&active, "Active").await;
    assert_eq!(status, StatusCode::CREATED, "{joined}");
    assert_eq!(joined["team"]["status"], "JOINED");
    assert_eq!(role_in_team(&joined).await.as_deref(), Some("MEMBER"));

    // An invitation without a Team says nothing about one.
    let (_, plain) = invite("plain@penfii.test", Value::Null).await;
    let (_, plain_joined, _) = accept(&plain, "Plain").await;
    assert!(plain_joined.get("team").is_none(), "{plain_joined}");

    // Archived after the invitation was made: the invitation stays valid, the person joins the Workspace only,
    // and both the response and the audit log say so. An archived Team is not offered for a new invitation.
    let (_, pending) = invite("late@penfii.test", json!(team_id)).await;
    let (_, waiting) = invite("waiting@penfii.test", json!(team_id)).await;
    assert_eq!(
        env.send(
            &env.owner,
            "POST",
            &env.url(&format!("teams/{team_id}/archive")),
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    let (status, body) = invite("new@penfii.test", json!(team_id)).await;
    assert_eq!(
        (status, body["code"].as_str()),
        (StatusCode::CONFLICT, Some("team_archived")),
        "{body}"
    );
    let (_, listed) = env.get(&env.owner, &env.url("invitations")).await;
    assert_eq!(listed["items"][1]["teamStatus"], "ARCHIVED");
    let (status, late, _) = accept(&pending, "Late").await;
    assert_eq!(status, StatusCode::CREATED, "{late}");
    assert_eq!(
        (
            late["team"]["status"].as_str(),
            late["team"]["teamName"].as_str()
        ),
        (Some("ARCHIVED"), Some("Core"))
    );
    assert_eq!(role_in_team(&late).await, None);
    let (status, _) = env.get(&env.owner, &env.url("members")).await;
    assert_eq!(status, StatusCode::OK);
    let skipped = env.audit_actions().await;
    assert!(
        skipped.contains(&"invitation.team_skipped".to_string()),
        "{skipped:?}"
    );

    // Restored before it is accepted: the Team step resumes.
    assert_eq!(
        env.send(
            &env.owner,
            "POST",
            &env.url(&format!("teams/{team_id}/restore")),
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    let (_, resumed, _) = accept(&waiting, "Waiting").await;
    assert_eq!(resumed["team"]["status"], "JOINED");
    assert_eq!(role_in_team(&resumed).await.as_deref(), Some("MEMBER"));

    // A Team that no longer exists: Workspace only, reported by the name it had.
    let other = env.team("Gone soon", &lead).await;
    let (_, gone) = invite("gone@penfii.test", json!(other)).await;
    sqlx::query("DELETE FROM penfii.team_members WHERE team_id = $1")
        .bind(other.parse::<Uuid>().unwrap())
        .execute(&env.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM penfii.teams WHERE id = $1")
        .bind(other.parse::<Uuid>().unwrap())
        .execute(&env.pool)
        .await
        .unwrap();
    let (_, listed) = env.get(&env.owner, &env.url("invitations")).await;
    assert_eq!(listed["items"][0]["teamStatus"], "GONE");
    let (status, vanished, _) = accept(&gone, "Gone").await;
    assert_eq!(status, StatusCode::CREATED, "{vanished}");
    assert_eq!(vanished["team"]["status"], "GONE");
    assert_eq!(vanished["team"]["teamName"], "Gone soon");
}

#[sqlx::test(migrations = false)]
async fn an_archived_team_keeps_its_work_view_and_cannot_be_newly_linked_or_given_a_new_role(
    pool: PgPool,
) {
    let env = setup(pool).await;
    let lead = env.person("lead", "MEMBER").await;
    let linked_only = env.team("Linked only", &lead).await;
    let with_role = env.team("With role", &lead).await;
    let late = env.team("Late", &lead).await;
    let (status, project, _) = call(
        &env.app,
        "POST",
        &env.url("projects"),
        Some(&env.owner.cookie),
        Some(json!({"name": "Shared", "teamIds": [linked_only, with_role]})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    let project_id = project["id"].as_str().unwrap().to_string();
    let participation =
        |team: &str| format!("/v1/projects/{project_id}/teams/{team}/participation");
    assert_eq!(
        env.send(
            &env.owner,
            "PUT",
            &participation(&with_role),
            Some(json!({"role": "EDITOR"}))
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    for team in [&linked_only, &with_role, &late] {
        assert_eq!(
            env.send(
                &env.owner,
                "POST",
                &env.url(&format!("teams/{team}/archive")),
                None
            )
            .await
            .0,
            StatusCode::OK
        );
    }

    // Ticket and work views of an Archived Team are unchanged: still readable.
    let (status, view) = env
        .get(
            &env.owner,
            &env.url(&format!("work?scope=TEAM&teamId={with_role}&lens=ALL")),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{view}");
    let (status, views) = env
        .get(
            &env.owner,
            &env.url(&format!("views?targetKind=TEAM&teamId={with_role}")),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{views}");

    // The Project's permissions page tags the archived Teams and keeps their roles.
    let (status, permissions) = env
        .get(
            &env.owner,
            &format!("/v1/projects/{project_id}/permissions"),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{permissions}");
    let teams = permissions["teams"].as_array().unwrap();
    assert_eq!(teams.len(), 2);
    assert!(
        teams.iter().all(|team| team["archivedAt"].is_string()),
        "{teams:?}"
    );
    assert_eq!(
        teams
            .iter()
            .find(|team| team["teamId"] == json!(with_role))
            .unwrap()["role"],
        "EDITOR"
    );

    // No new link and no role that the Team did not have.
    let (status, _) = env
        .send(
            &env.owner,
            "PUT",
            &format!("/v1/projects/{project_id}/teams/{late}"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, body) = env
        .send(
            &env.owner,
            "PUT",
            &participation(&linked_only),
            Some(json!({"role": "VIEWER"})),
        )
        .await;
    assert_eq!(
        (status, body["code"].as_str()),
        (StatusCode::CONFLICT, Some("team_archived")),
        "{body}"
    );
    // An existing participation may still be lowered, then ended and unlinked by the Project admin.
    for role in [json!({"role": "VIEWER"}), json!({"role": null})] {
        assert_eq!(
            env.send(&env.owner, "PUT", &participation(&with_role), Some(role))
                .await
                .0,
            StatusCode::NO_CONTENT
        );
    }
    assert_eq!(
        env.send(
            &env.owner,
            "DELETE",
            &format!("/v1/projects/{project_id}/teams/{with_role}"),
            None
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    // The department placement does not change while archived.
    let (status, body) = env
        .send(
            &env.owner,
            "PUT",
            &env.url(&format!("teams/{late}/department")),
            Some(json!({"departmentId": null})),
        )
        .await;
    assert_eq!(
        (status, body["code"].as_str()),
        (StatusCode::CONFLICT, Some("team_archived")),
        "{body}"
    );
}

#[sqlx::test(migrations = false)]
async fn deleting_a_department_moves_its_teams_and_sub_departments_up_one_level(pool: PgPool) {
    let env = setup(pool).await;
    let lead = env.person("lead", "MEMBER").await;
    let member = env.person("member", "MEMBER").await;
    let department = |name: &str, parent: Option<&str>| json!({"name": name, "parentId": parent});
    let (_, top) = env
        .send(
            &env.owner,
            "POST",
            &env.url("departments"),
            Some(department("Product", None)),
        )
        .await;
    let top = top["id"].as_str().unwrap().to_string();
    let (_, middle) = env
        .send(
            &env.owner,
            "POST",
            &env.url("departments"),
            Some(department("Dev", Some(&top))),
        )
        .await;
    let middle = middle["id"].as_str().unwrap().to_string();
    let (_, leaf) = env
        .send(
            &env.owner,
            "POST",
            &env.url("departments"),
            Some(department("Infra", Some(&middle))),
        )
        .await;
    let leaf = leaf["id"].as_str().unwrap().to_string();
    let live = env.team("Backend", &lead).await;
    let archived = env.team("Old", &lead).await;
    for (team, place) in [(&live, &middle), (&archived, &middle)] {
        let (status, body) = env
            .send(
                &env.owner,
                "PUT",
                &env.url(&format!("teams/{team}/department")),
                Some(json!({"departmentId": place})),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    let (status, body) = env
        .send(
            &env.owner,
            "POST",
            &env.url(&format!("teams/{archived}/archive")),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Only an owner or admin deletes a department.
    let path = env.url(&format!("departments/{middle}"));
    assert_eq!(
        env.send(&member, "DELETE", &path, None).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        env.send(&env.owner, "DELETE", &path, None).await.0,
        StatusCode::NO_CONTENT
    );

    // The Teams (the archived one too) and the sub-department sit under the parent now; nothing else changed.
    assert_eq!(
        env.team_state(&member, &live).await["departmentId"],
        json!(top)
    );
    assert_eq!(
        env.team_state(&member, &archived).await["departmentId"],
        json!(top)
    );
    assert!(env.team_state(&member, &archived).await["archivedAt"].is_string());
    let (_, listed) = env.get(&member, &env.url("departments")).await;
    let leaf_row = listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == json!(leaf))
        .unwrap()
        .clone();
    assert_eq!(leaf_row["parentId"], json!(top));
    assert!(
        listed["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["id"] != json!(middle))
    );

    // A top-level department that is deleted leaves its Teams unplaced.
    let (_, root) = env
        .send(
            &env.owner,
            "POST",
            &env.url("departments"),
            Some(department("Ops", None)),
        )
        .await;
    let root = root["id"].as_str().unwrap().to_string();
    env.send(
        &env.owner,
        "PUT",
        &env.url(&format!("teams/{live}/department")),
        Some(json!({"departmentId": root})),
    )
    .await;
    assert_eq!(
        env.send(
            &env.owner,
            "DELETE",
            &env.url(&format!("departments/{root}")),
            None
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        env.team_state(&member, &live).await["departmentId"],
        Value::Null
    );
    let actions = env.audit_actions().await;
    for expected in ["department.created", "department.deleted", "team.placed"] {
        assert!(
            actions.contains(&expected.to_string()),
            "{expected} {actions:?}"
        );
    }
}

#[sqlx::test(migrations = false)]
async fn a_team_can_be_created_with_a_description(pool: PgPool) {
    let env = setup(pool).await;
    let lead = env.person("lead", "MEMBER").await;
    let (status, team, _) = call(
        &env.app,
        "POST",
        &env.url("teams"),
        Some(&env.owner.cookie),
        Some(json!({"name": "Growth", "description": "  Runs experiments  ", "members": [{"userId": lead.id, "role": "MANAGER"}]})),
    )
    .await;
    assert_eq!(
        (status, &team["description"]),
        (StatusCode::CREATED, &json!("Runs experiments")),
        "{team}"
    );
    let id = team["id"].as_str().unwrap().to_string();
    assert_eq!(
        env.team_state(&lead, &id).await["description"],
        json!("Runs experiments")
    );
    // Without one, or with only blanks, the Team has none.
    let (_, plain, _) = call(
        &env.app,
        "POST",
        &env.url("teams"),
        Some(&env.owner.cookie),
        Some(json!({"name": "Plain", "description": "   ", "members": [{"userId": lead.id, "role": "MANAGER"}]})),
    )
    .await;
    assert_eq!(plain["description"], Value::Null);
}
