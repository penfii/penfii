//! Changing the e-mail address (FEA-140): the new address is confirmed from the new address before it
//! is the account's, the old one works until then, a server without e-mail refuses the self-service
//! change, and the operator's change leaves the address unverified.

use std::time::Duration;

use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;

use crate::common::smtp::{FakeSmtp, Mode, Received};
use crate::common::{self, MEMBER_PASSWORD as PASSWORD, call, login};

async fn with_mail(pool: PgPool) -> (common::Env, FakeSmtp) {
    let smtp = FakeSmtp::start(Mode::Accept).await;
    let env = common::Env::with_mailer(pool, smtp.mailer(Duration::from_secs(5))).await;
    (env, smtp)
}

async fn ask(env: &common::Env, who: &common::Person, email: &str) -> common::Reply {
    env.call(
        who,
        "POST",
        "/v1/me/email-change",
        Some(json!({"email": email})),
    )
    .await
}

async fn confirm(env: &common::Env, token: &str) -> common::Reply {
    call(
        &env.app,
        "POST",
        "/v1/auth/email-change/confirm",
        None,
        Some(json!({"token": token})),
    )
    .await
}

fn mails_with(smtp: &FakeSmtp, needle: &str) -> Vec<Received> {
    smtp.received()
        .into_iter()
        .filter(|mail| mail.text().contains(needle))
        .collect()
}

fn token_of(mail: &Received) -> String {
    let text = mail.text();
    let link = text
        .split_whitespace()
        .find(|word| word.starts_with("https://penfii.example.com/confirm-email"))
        .unwrap_or_else(|| panic!("no link in {text}"));
    assert!(!link.contains('?'), "the token must not be in a query");
    link.split_once('#').expect("fragment").1.to_string()
}

async fn verified(pool: &PgPool, id: uuid::Uuid) -> bool {
    sqlx::query_scalar("SELECT email_verified_at IS NOT NULL FROM penfii.users WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn without_smtp_the_self_service_change_is_refused_and_nothing_changes(pool: PgPool) {
    let env = common::Env::new(pool.clone()).await;
    let member = env.person("mina", "MEMBER").await;
    let asked = ask(&env, &member, "mina2@penfii.test").await;
    assert_eq!(asked.status, StatusCode::CONFLICT, "{}", asked.body);
    assert_eq!(asked.body["code"], "email_change_unavailable");
    // The profile form cannot slip a new address in either.
    let patched = env
        .call(
            &member,
            "PATCH",
            "/v1/me",
            Some(json!({"displayName": "Mina", "email": "mina2@penfii.test"})),
        )
        .await;
    assert_eq!(patched.status, StatusCode::CONFLICT, "{}", patched.body);
    assert_eq!(patched.body["code"], "email_change_requires_verification");
    let me = env.call(&member, "GET", "/v1/me", None).await;
    assert_eq!(me.body["email"], "mina@penfii.test");
    // The same address (another letter case) and a new name are an ordinary profile change.
    let patched = env
        .call(
            &member,
            "PATCH",
            "/v1/me",
            Some(json!({"displayName": "Mina K", "email": "Mina@Penfii.test"})),
        )
        .await;
    assert_eq!(patched.status, StatusCode::OK, "{}", patched.body);
    assert_eq!(patched.body["displayName"], "Mina K");
    assert!(verified(&pool, member.id).await);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn the_new_address_counts_only_after_its_link_is_opened(pool: PgPool) {
    let (env, smtp) = with_mail(pool.clone()).await;
    let member = env.person("mina", "MEMBER").await;

    let asked = ask(&env, &member, "New.Mina@Penfii.test").await;
    assert_eq!(asked.status, StatusCode::ACCEPTED, "{}", asked.body);
    assert_eq!(asked.body["email"], "New.Mina@Penfii.test");
    let mails = mails_with(&smtp, "/confirm-email");
    assert_eq!(mails.len(), 1);
    assert_eq!(mails[0].to.to_lowercase(), "new.mina@penfii.test");
    let token = token_of(&mails[0]);
    // Only a hash is stored.
    let stored: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.email_change_requests WHERE encode(token_hash, 'escape') = $1",
    )
    .bind(&token)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(stored, 0);

    // Waiting: the old address signs in and is the account's, the new one is not.
    let pending = env.call(&member, "GET", "/v1/me/email-change", None).await;
    assert_eq!(pending.body["email"], "New.Mina@Penfii.test");
    assert_eq!(
        login(&env.app, "mina@penfii.test", PASSWORD).await.0,
        StatusCode::OK
    );
    assert_eq!(
        login(&env.app, "new.mina@penfii.test", PASSWORD).await.0,
        StatusCode::UNAUTHORIZED
    );
    let me = env.call(&member, "GET", "/v1/me", None).await;
    assert_eq!(me.body["email"], "mina@penfii.test");

    // A wrong link does nothing; the right one changes the address and the account stays verified.
    let wrong = confirm(&env, &"0".repeat(64)).await;
    assert_eq!(wrong.status, StatusCode::CONFLICT);
    assert_eq!(wrong.body["code"], "email_change_token_invalid");
    let done = confirm(&env, &token).await;
    assert_eq!(done.status, StatusCode::NO_CONTENT, "{}", done.body);
    assert_eq!(
        login(&env.app, "new.mina@penfii.test", PASSWORD).await.0,
        StatusCode::OK
    );
    assert_eq!(
        login(&env.app, "mina@penfii.test", PASSWORD).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert!(verified(&pool, member.id).await);
    assert_eq!(
        env.call(&member, "GET", "/v1/me/email-change", None)
            .await
            .body,
        serde_json::Value::Null
    );
    // The link works once, and the old address is told.
    assert_eq!(confirm(&env, &token).await.status, StatusCode::CONFLICT);
    for _ in 0..100 {
        if mails_with(&smtp, "changed")
            .iter()
            .any(|m| m.to == "mina@penfii.test")
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        smtp.received()
            .iter()
            .any(|mail| mail.to == "mina@penfii.test"
                && !mail.text().contains("/confirm-email")
                && !mail.text().contains("/invite/")),
        "the old address was not told"
    );
    // The audit log names the account and the way, never an address or the token.
    let audit: Vec<(String, serde_json::Value)> = sqlx::query_as(
        "SELECT action_code, data FROM penfii.audit_entries WHERE action_code LIKE 'user.email_change%' ORDER BY occurred_at, id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(audit.len(), 2, "{audit:?}");
    assert!(!audit.iter().any(|(_, data)| data.to_string().contains('@')));
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_link_that_expired_was_replaced_or_whose_address_was_taken_is_refused(pool: PgPool) {
    let (env, smtp) = with_mail(pool.clone()).await;
    let member = env.person("mina", "MEMBER").await;

    ask(&env, &member, "first@penfii.test").await;
    ask(&env, &member, "second@penfii.test").await;
    let mails = mails_with(&smtp, "/confirm-email");
    assert_eq!(mails.len(), 2);
    // The newer request replaced the older one.
    assert_eq!(
        confirm(&env, &token_of(&mails[0])).await.status,
        StatusCode::CONFLICT
    );
    // Expired.
    sqlx::query("UPDATE penfii.email_change_requests SET expires_at = now() - interval '1 minute'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        confirm(&env, &token_of(&mails[1])).await.status,
        StatusCode::CONFLICT
    );
    assert_eq!(
        env.call(&member, "GET", "/v1/me/email-change", None)
            .await
            .body,
        serde_json::Value::Null
    );

    // Taken by another account after the request was made.
    ask(&env, &member, "late@penfii.test").await;
    let token = token_of(mails_with(&smtp, "/confirm-email").last().unwrap());
    env.person("late", "MEMBER").await;
    let taken = confirm(&env, &token).await;
    assert_eq!(taken.status, StatusCode::CONFLICT);
    assert_eq!(taken.body["code"], "account_exists");
    let me = env.call(&member, "GET", "/v1/me", None).await;
    assert_eq!(me.body["email"], "mina@penfii.test");

    // An address another account already has is refused when asked for, and mails nothing.
    let before = smtp.received().len();
    let dup = ask(&env, &member, "owner@penfii.test").await;
    assert_eq!(dup.status, StatusCode::CONFLICT);
    assert_eq!(dup.body["code"], "account_exists");
    assert_eq!(smtp.received().len(), before);
    // Its own address is not a change.
    let same = ask(&env, &member, "MINA@penfii.test").await;
    assert_eq!(same.status, StatusCode::BAD_REQUEST, "{}", same.body);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn asking_needs_a_recent_authentication_and_is_limited(pool: PgPool) {
    let (env, smtp) = with_mail(pool.clone()).await;
    let member = env.person("mina", "MEMBER").await;
    sqlx::query(
        "UPDATE penfii.sessions SET last_reauth_at = now() - interval '2 hours' WHERE user_id = $1",
    )
    .bind(member.id)
    .execute(&pool)
    .await
    .unwrap();
    let stale = ask(&env, &member, "x@penfii.test").await;
    assert_eq!(stale.status, StatusCode::FORBIDDEN, "{}", stale.body);
    assert_eq!(stale.body["code"], "reauth_required");
    let before = smtp.received().len();
    assert_eq!(before, 1, "only the invitation was mailed");

    let again = env
        .call(
            &member,
            "POST",
            "/v1/me/reauth/password",
            Some(json!({"password": PASSWORD})),
        )
        .await;
    assert_eq!(again.status, StatusCode::NO_CONTENT);
    // One address gets three mails an hour, however many accounts ask.
    for round in 0..5 {
        let reply = ask(&env, &member, "one-address@penfii.test").await;
        if round < 3 {
            assert_eq!(reply.status, StatusCode::ACCEPTED, "{}", reply.body);
        } else {
            assert_eq!(
                reply.status,
                StatusCode::TOO_MANY_REQUESTS,
                "{}",
                reply.body
            );
        }
    }
    assert_eq!(mails_with(&smtp, "/confirm-email").len(), 3);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_change_also_ends_the_reset_links_of_the_old_address(pool: PgPool) {
    let (env, smtp) = with_mail(pool.clone()).await;
    let member = env.person("mina", "MEMBER").await;
    let asked = call(
        &env.app,
        "POST",
        "/v1/auth/password-reset/request",
        None,
        Some(json!({"email": "mina@penfii.test"})),
    )
    .await;
    assert_eq!(asked.status, StatusCode::ACCEPTED);
    for _ in 0..100 {
        if !mails_with(&smtp, "/reset-password").is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let reset = mails_with(&smtp, "/reset-password").remove(0);
    let reset_token = reset
        .text()
        .split_whitespace()
        .find(|word| word.contains("/reset-password"))
        .unwrap()
        .split_once('#')
        .unwrap()
        .1
        .to_string();
    ask(&env, &member, "elsewhere@penfii.test").await;
    let token = token_of(mails_with(&smtp, "/confirm-email").last().unwrap());
    assert_eq!(confirm(&env, &token).await.status, StatusCode::NO_CONTENT);
    let used = call(
        &env.app,
        "POST",
        "/v1/auth/password-reset/confirm",
        None,
        Some(json!({"token": reset_token, "password": "a brand new passphrase"})),
    )
    .await;
    assert_eq!(used.status, StatusCode::CONFLICT, "{}", used.body);
    assert_eq!(used.body["code"], "reset_token_invalid");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn the_operators_change_is_unverified_until_the_person_confirms_it(pool: PgPool) {
    let (env, smtp) = with_mail(pool.clone()).await;
    let member = env.person("mina", "MEMBER").await;
    // Unknown account, a taken address and the same address are refused.
    assert!(
        penfii_core::identity::operator_set_email(&pool, "nobody@penfii.test", "n@penfii.test")
            .await
            .is_err()
    );
    assert!(
        penfii_core::identity::operator_set_email(&pool, "mina@penfii.test", "owner@penfii.test")
            .await
            .is_err()
    );
    assert!(
        penfii_core::identity::operator_set_email(&pool, "mina@penfii.test", "Mina@penfii.test")
            .await
            .is_err()
    );

    penfii_core::identity::operator_set_email(&pool, "mina@penfii.test", "mina.new@penfii.test")
        .await
        .unwrap();
    assert!(!verified(&pool, member.id).await);
    // She signs in with the new address and her session stays.
    assert_eq!(
        login(&env.app, "mina.new@penfii.test", PASSWORD).await.0,
        StatusCode::OK
    );
    assert_eq!(
        env.call(&member, "GET", "/v1/me", None).await.status,
        StatusCode::OK
    );
    // Nothing is mailed to an address nobody has proved: no reset link.
    let before = smtp.received().len();
    call(
        &env.app,
        "POST",
        "/v1/auth/password-reset/request",
        None,
        Some(json!({"email": "mina.new@penfii.test"})),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(smtp.received().len(), before);
    // She confirms it from her account: the same address is a valid request while it is unverified.
    let asked = ask(&env, &member, "mina.new@penfii.test").await;
    assert_eq!(asked.status, StatusCode::ACCEPTED, "{}", asked.body);
    let token = token_of(mails_with(&smtp, "/confirm-email").last().unwrap());
    assert_eq!(confirm(&env, &token).await.status, StatusCode::NO_CONTENT);
    assert!(verified(&pool, member.id).await);
    let audit: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.audit_entries WHERE action_code = 'user.email_set' AND actor_user_id IS NULL",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(audit, 1);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn whether_an_address_is_taken_is_answered_only_within_the_limit(pool: PgPool) {
    let (env, _smtp) = with_mail(pool.clone()).await;
    let member = env.person("mina", "MEMBER").await;
    // Five asks an hour per account, counted before the address is looked up: probing which
    // addresses have an account runs into the same limit as asking for mails.
    env.person("taken", "MEMBER").await;
    for round in 0..6 {
        // Two taken addresses, so the per-address limit (three) is not the one reached.
        let address = ["owner@penfii.test", "taken@penfii.test"][round % 2];
        let reply = ask(&env, &member, address).await;
        if round < 5 {
            assert_eq!(reply.status, StatusCode::CONFLICT, "{}", reply.body);
            assert_eq!(reply.body["code"], "account_exists");
        } else {
            assert_eq!(
                reply.status,
                StatusCode::TOO_MANY_REQUESTS,
                "{}",
                reply.body
            );
        }
    }
}
