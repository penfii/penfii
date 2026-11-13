//! Invitation links and their optional e-mail delivery (FEA-062): the link works without e-mail, an
//! e-mail has one open invitation which is re-issued, and a failed e-mail never fails the invitation.

use std::time::Duration;

use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::common::smtp::{FakeSmtp, Mode};
use crate::common::{self, MEMBER_PASSWORD as PASSWORD, call};

fn invitations_url(env: &common::Env) -> String {
    format!("/v1/workspaces/{}/invitations", env.ws)
}

async fn preview(env: &common::Env, token: &Value) -> StatusCode {
    call(
        &env.app,
        "POST",
        "/v1/invitations/preview",
        None,
        Some(json!({"token": token})),
    )
    .await
    .status
}

async fn listed(env: &common::Env) -> Vec<Value> {
    let reply = env
        .call(&env.owner, "GET", &invitations_url(env), None)
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    reply.body["items"].as_array().unwrap().clone()
}

/// `who` sends the invitation `id` again.
async fn reissue_as(env: &common::Env, who: &common::Person, id: &Value) -> common::Reply {
    env.call(
        who,
        "POST",
        &format!("{}/{}/reissue", invitations_url(env), id.as_str().unwrap()),
        None,
    )
    .await
}

async fn reissue(env: &common::Env, id: &Value) -> common::Reply {
    reissue_as(env, &env.owner, id).await
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn without_smtp_an_invitation_is_a_link_that_works(pool: PgPool) {
    let env = common::Env::new(pool).await;
    let sent = env.invite(&env.owner, "guest@penfii.test", "MEMBER").await;
    assert_eq!(sent.status, StatusCode::CREATED, "{}", sent.body);
    assert_eq!(sent.body["emailStatus"], "NOT_CONFIGURED");
    assert_eq!(sent.body["reissued"], false);
    let token = sent.body["inviteToken"].as_str().unwrap();
    assert!(token.len() >= 32);

    let items = listed(&env).await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["status"], "PENDING");
    assert_eq!(items[0]["emailStatus"], "NOT_CONFIGURED");
    // The link is not listed, and no feature says the server sends e-mail.
    assert!(!items[0].to_string().contains(token));
    let info = call(&env.app, "GET", "/v1/server-info", None, None).await;
    assert!(
        !info.body["features"]
            .as_array()
            .unwrap()
            .contains(&json!("email"))
    );

    let accepted = call(
        &env.app,
        "POST",
        "/v1/invitations/accept",
        None,
        Some(json!({"token": token, "displayName": "Guest", "password": PASSWORD})),
    )
    .await;
    assert_eq!(accepted.status, StatusCode::CREATED, "{}", accepted.body);
    assert_eq!(listed(&env).await[0]["status"], "ACCEPTED");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn inviting_the_same_address_again_reissues_the_open_invitation(pool: PgPool) {
    let env = common::Env::new(pool).await;
    let first = env.invite(&env.owner, "guest@penfii.test", "MEMBER").await;
    assert_eq!(first.status, StatusCode::CREATED);
    // The second request changes the role and writes the address in another case.
    let second = env.invite(&env.owner, "Guest@Penfii.test", "ADMIN").await;
    assert_eq!(second.status, StatusCode::OK, "{}", second.body);
    assert_eq!(second.body["reissued"], true);
    assert_eq!(second.body["id"], first.body["id"]);
    assert_eq!(second.body["role"], "ADMIN");
    assert_ne!(second.body["inviteToken"], first.body["inviteToken"]);

    // The first link is dead at once, the second one works.
    assert_eq!(
        preview(&env, &first.body["inviteToken"]).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        preview(&env, &second.body["inviteToken"]).await,
        StatusCode::OK
    );
    assert_eq!(listed(&env).await.len(), 1);

    // An expired invitation is not an open one: inviting again makes a new invitation, and the old
    // one stays behind as expired.
    sqlx::query("UPDATE penfii.workspace_invitations SET expires_at = now() - interval '1 day'")
        .execute(&env.pool)
        .await
        .unwrap();
    assert_eq!(listed(&env).await[0]["status"], "EXPIRED");
    let third = env.invite(&env.owner, "guest@penfii.test", "MEMBER").await;
    assert_eq!(third.status, StatusCode::CREATED, "{}", third.body);
    assert_ne!(third.body["id"], first.body["id"]);
    let statuses: Vec<String> = listed(&env)
        .await
        .iter()
        .map(|item| item["status"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(statuses, ["PENDING", "EXPIRED"]);
    let actions: Vec<String> = sqlx::query_scalar(
        "SELECT action_code FROM penfii.audit_entries WHERE action_code LIKE 'invitation.%' ORDER BY occurred_at, id",
    )
    .fetch_all(&env.pool)
    .await
    .unwrap();
    assert_eq!(
        actions,
        [
            "invitation.created",
            "invitation.reissued",
            "invitation.created"
        ]
    );

    // A revoked invitation is closed: inviting again makes a new one.
    let id = third.body["id"].clone();
    let revoked = env
        .call(
            &env.owner,
            "DELETE",
            &format!("{}/{}", invitations_url(&env), id.as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(revoked.status, StatusCode::NO_CONTENT);
    let fourth = env.invite(&env.owner, "guest@penfii.test", "MEMBER").await;
    assert_eq!(fourth.status, StatusCode::CREATED);
    assert_ne!(fourth.body["id"], id);
    assert_eq!(listed(&env).await.len(), 3);
    // Neither a revoked nor an accepted invitation can be sent again.
    assert_eq!(reissue(&env, &id).await.status, StatusCode::CONFLICT);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn the_database_allows_one_open_invitation_per_address(pool: PgPool) {
    let env = common::Env::new(pool).await;
    let first = env.invite(&env.owner, "guest@penfii.test", "MEMBER").await;
    assert_eq!(first.status, StatusCode::CREATED);
    let duplicate = sqlx::query(
        "INSERT INTO penfii.workspace_invitations (id, workspace_id, email_key, role, token_hash, expires_at, created_by)
         VALUES ($1, $2, 'guest@penfii.test', 'MEMBER', $3, now() + interval '1 day', $4)",
    )
    .bind(Uuid::new_v4())
    .bind(env.ws)
    .bind(vec![1u8; 32])
    .bind(env.owner.id)
    .execute(&env.pool)
    .await;
    assert!(duplicate.is_err(), "a second open invitation was stored");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn simultaneous_invitations_of_one_address_end_as_one_row(pool: PgPool) {
    let env = common::Env::new(pool).await;
    let calls = (0..8).map(|_| env.invite(&env.owner, "race@penfii.test", "MEMBER"));
    let replies = futures_util::future::join_all(calls).await;
    let mut tokens = Vec::new();
    for reply in &replies {
        assert!(
            matches!(reply.status, StatusCode::CREATED | StatusCode::OK),
            "{} {}",
            reply.status,
            reply.body
        );
        tokens.push(reply.body["inviteToken"].clone());
    }
    assert_eq!(listed(&env).await.len(), 1);
    // Exactly one of the links is the current one.
    let mut working = 0;
    for token in &tokens {
        if preview(&env, token).await == StatusCode::OK {
            working += 1;
        }
    }
    assert_eq!(working, 1);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn an_admin_does_not_reissue_an_invitation_for_an_admin(pool: PgPool) {
    let env = common::Env::new(pool).await;
    let admin = env.person("admin", "ADMIN").await;
    let member = env.person("member", "MEMBER").await;
    let admin_invite = env
        .invite(&env.owner, "future-admin@penfii.test", "ADMIN")
        .await;
    let plain_invite = env.invite(&env.owner, "plain@penfii.test", "MEMBER").await;
    assert_eq!(
        reissue_as(&env, &admin, &admin_invite.body["id"])
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    // Not by inviting the same address with the plain role either.
    assert_eq!(
        env.invite(&admin, "future-admin@penfii.test", "MEMBER")
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        reissue_as(&env, &member, &plain_invite.body["id"])
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        reissue_as(&env, &admin, &plain_invite.body["id"])
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        reissue(&env, &admin_invite.body["id"]).await.status,
        StatusCode::OK
    );
    assert_eq!(
        reissue(&env, &json!(Uuid::new_v4().to_string()))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn with_smtp_the_same_link_is_mailed_in_the_server_language(pool: PgPool) {
    let smtp = FakeSmtp::start(Mode::Accept).await;
    let env = common::Env::with_mailer(pool, smtp.mailer(Duration::from_secs(5))).await;
    let info = call(&env.app, "GET", "/v1/server-info", None, None).await;
    assert!(
        info.body["features"]
            .as_array()
            .unwrap()
            .contains(&json!("email"))
    );

    let sent = env.invite(&env.owner, "Guest@Penfii.test", "MEMBER").await;
    assert_eq!(sent.status, StatusCode::CREATED, "{}", sent.body);
    assert_eq!(sent.body["emailStatus"], "SENT");
    let token = sent.body["inviteToken"].as_str().unwrap();
    let mails = smtp.received();
    assert_eq!(mails.len(), 1);
    assert_eq!(mails[0].to, "guest@penfii.test");
    // The link starts with the public origin; the default language is Korean.
    assert!(
        mails[0]
            .text()
            .contains(&format!("https://penfii.example.com/invite/{token}")),
        "{}",
        mails[0].data
    );
    assert!(mails[0].text().contains("초대"));
    let items = listed(&env).await;
    assert_eq!(items[0]["emailStatus"], "SENT");
    assert!(items[0]["emailAttemptedAt"].is_string());
    assert!(items[0]["emailError"].is_null());

    // With the server language set to English the mail is English.
    sqlx::query("UPDATE penfii.instance_settings SET default_locale = 'en-US'")
        .execute(&env.pool)
        .await
        .unwrap();
    let again = env.invite(&env.owner, "other@penfii.test", "MEMBER").await;
    assert_eq!(again.body["emailStatus"], "SENT");
    assert!(smtp.received()[1].text().contains("invited you"));

    // Neither the audit log nor the invitation row keeps the link.
    let stored: String = sqlx::query_scalar(
        "SELECT COALESCE(string_agg(data::text, ' '), '') FROM penfii.audit_entries",
    )
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert!(!stored.contains(token));
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_failed_mail_leaves_a_working_invitation_that_can_be_sent_again(pool: PgPool) {
    let smtp = FakeSmtp::start(Mode::Refuse).await;
    let env = common::Env::with_mailer(pool, smtp.mailer(Duration::from_secs(5))).await;
    let sent = env.invite(&env.owner, "guest@penfii.test", "MEMBER").await;
    assert_eq!(sent.status, StatusCode::CREATED, "{}", sent.body);
    assert_eq!(sent.body["emailStatus"], "FAILED");
    let error = sent.body["emailError"].as_str().unwrap();
    assert_eq!(error, "the SMTP server refused the message");
    assert!(!error.contains("127.0.0.1") && !error.contains("550"));

    // The invitation itself is untouched by the failure.
    let items = listed(&env).await;
    assert_eq!(items[0]["status"], "PENDING");
    assert_eq!(items[0]["emailStatus"], "FAILED");
    assert_eq!(items[0]["emailError"], error);
    assert_eq!(
        preview(&env, &sent.body["inviteToken"]).await,
        StatusCode::OK
    );

    // The server recovers: "send again" gives a new link, mails it and kills the old one.
    smtp.set(Mode::Accept);
    let retried = reissue(&env, &sent.body["id"]).await;
    assert_eq!(retried.status, StatusCode::OK, "{}", retried.body);
    assert_eq!(retried.body["emailStatus"], "SENT");
    assert_eq!(retried.body["reissued"], true);
    assert_eq!(
        preview(&env, &sent.body["inviteToken"]).await,
        StatusCode::NOT_FOUND
    );
    let token = retried.body["inviteToken"].as_str().unwrap();
    assert_eq!(smtp.received().len(), 1);
    assert!(smtp.received()[0].text().contains(token));
    let accepted = call(
        &env.app,
        "POST",
        "/v1/invitations/accept",
        None,
        Some(json!({"token": token, "displayName": "Guest", "password": PASSWORD})),
    )
    .await;
    assert_eq!(accepted.status, StatusCode::CREATED);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_server_that_does_not_answer_fails_the_mail_in_time_and_not_the_invitation(pool: PgPool) {
    let smtp = FakeSmtp::start(Mode::Hang).await;
    let env = common::Env::with_mailer(pool, smtp.mailer(Duration::from_secs(1))).await;
    let started = std::time::Instant::now();
    let sent = env.invite(&env.owner, "guest@penfii.test", "MEMBER").await;
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(sent.status, StatusCode::CREATED, "{}", sent.body);
    assert_eq!(sent.body["emailStatus"], "FAILED");
    assert_eq!(
        sent.body["emailError"],
        "the SMTP server did not answer in time"
    );
    assert_eq!(
        preview(&env, &sent.body["inviteToken"]).await,
        StatusCode::OK
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_late_result_does_not_overwrite_the_state_of_a_newer_link(pool: PgPool) {
    let smtp = FakeSmtp::start(Mode::Accept).await;
    // The first connection hangs until the mailer gives up (2 s); the next one is quick.
    smtp.next(Mode::Hang);
    let env = common::Env::with_mailer(pool, smtp.mailer(Duration::from_secs(2))).await;
    let slow = env.invite(&env.owner, "guest@penfii.test", "MEMBER");
    let quick = async {
        tokio::time::sleep(Duration::from_millis(500)).await;
        env.invite(&env.owner, "guest@penfii.test", "MEMBER").await
    };
    let (slow, quick) = tokio::join!(slow, quick);
    assert_eq!(quick.body["emailStatus"], "SENT", "{}", quick.body);
    // The slow request reports its own failure to its caller ...
    assert_eq!(slow.body["emailStatus"], "FAILED");
    // ... but the stored state belongs to the link that is current now.
    let items = listed(&env).await;
    assert_eq!(items[0]["emailStatus"], "SENT");
    assert_eq!(
        preview(&env, &quick.body["inviteToken"]).await,
        StatusCode::OK
    );
    assert_eq!(
        preview(&env, &slow.body["inviteToken"]).await,
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn an_expired_invitation_does_not_stand_in_the_way_of_a_new_one(pool: PgPool) {
    let env = common::Env::new(pool).await;
    let admin = env.person("admin", "ADMIN").await;
    let expire = || async {
        sqlx::query("UPDATE penfii.workspace_invitations SET expires_at = now() - interval '1 day' WHERE revoked_at IS NULL AND accepted_at IS NULL")
            .execute(&env.pool)
            .await
            .unwrap();
    };

    // An admin invitation that nobody used and that ran out.
    let old = env.invite(&env.owner, "later@penfii.test", "ADMIN").await;
    assert_eq!(old.status, StatusCode::CREATED);
    // While it is active, an admin still does not take it over.
    assert_eq!(
        env.invite(&admin, "later@penfii.test", "MEMBER")
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    expire().await;
    // An expired invitation cannot be revoked (it is not pending) ...
    let revoke = env
        .call(
            &env.owner,
            "DELETE",
            &format!(
                "{}/{}",
                invitations_url(&env),
                old.body["id"].as_str().unwrap()
            ),
            None,
        )
        .await;
    assert_eq!(revoke.status, StatusCode::CONFLICT);
    // ... and it no longer blocks a new invitation, not even one an admin makes for a plain member.
    let fresh = env.invite(&admin, "later@penfii.test", "MEMBER").await;
    assert_eq!(fresh.status, StatusCode::CREATED, "{}", fresh.body);
    assert_eq!(fresh.body["reissued"], false);
    assert_ne!(fresh.body["id"], old.body["id"]);
    assert_eq!(fresh.body["role"], "MEMBER");

    // The old one stays in the list as expired (not as revoked), the new one is pending.
    let items = listed(&env).await;
    let status_of = |id: &Value| {
        items
            .iter()
            .find(|item| item["id"] == *id)
            .map(|item| item["status"].clone())
    };
    assert_eq!(status_of(&old.body["id"]), Some(json!("EXPIRED")));
    assert_eq!(status_of(&fresh.body["id"]), Some(json!("PENDING")));
    assert_eq!(
        preview(&env, &old.body["inviteToken"]).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        preview(&env, &fresh.body["inviteToken"]).await,
        StatusCode::OK
    );
    // The database still holds one open row for the address, and it is the new one.
    let open: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM penfii.workspace_invitations WHERE email_key = 'later@penfii.test' AND accepted_at IS NULL AND revoked_at IS NULL",
    )
    .fetch_all(&env.pool)
    .await
    .unwrap();
    assert_eq!(
        open,
        [Uuid::parse_str(fresh.body["id"].as_str().unwrap()).unwrap()]
    );

    // The active invitation is re-issued as before, with the same permission rule.
    let again = env.invite(&admin, "later@penfii.test", "MEMBER").await;
    assert_eq!(
        (again.status, &again.body["id"], &again.body["reissued"]),
        (StatusCode::OK, &fresh.body["id"], &json!(true))
    );
    let owner_admin = env.invite(&env.owner, "later@penfii.test", "ADMIN").await;
    assert_eq!(owner_admin.status, StatusCode::OK);
    assert_eq!(
        env.invite(&admin, "later@penfii.test", "MEMBER")
            .await
            .status,
        StatusCode::FORBIDDEN
    );

    // "New link" on an expired invitation is still the way to send that very invitation again.
    expire().await;
    let revived = reissue(&env, &fresh.body["id"]).await;
    assert_eq!(revived.status, StatusCode::OK, "{}", revived.body);
    assert_eq!(
        preview(&env, &revived.body["inviteToken"]).await,
        StatusCode::OK
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn simultaneous_invitations_over_an_expired_one_end_as_one_open_row(pool: PgPool) {
    let env = common::Env::new(pool).await;
    env.invite(&env.owner, "race@penfii.test", "MEMBER").await;
    sqlx::query("UPDATE penfii.workspace_invitations SET expires_at = now() - interval '1 day'")
        .execute(&env.pool)
        .await
        .unwrap();
    let calls = (0..8).map(|_| env.invite(&env.owner, "race@penfii.test", "MEMBER"));
    for reply in futures_util::future::join_all(calls).await {
        assert!(
            matches!(reply.status, StatusCode::CREATED | StatusCode::OK),
            "{}",
            reply.body
        );
    }
    let open: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.workspace_invitations WHERE accepted_at IS NULL AND revoked_at IS NULL AND expires_at > now()",
    )
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(open, 1);
}
