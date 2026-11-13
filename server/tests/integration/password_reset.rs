//! Password reset by e-mail (FEA-063): one-use tokens that expire, the same answer for every account,
//! the reset counters, and what a reset ends.

use std::time::{Duration, Instant};

use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::common::smtp::{FakeSmtp, Mode};
use crate::common::{self, MEMBER_PASSWORD as PASSWORD, call, login};

const NEW_PASSWORD: &str = "a brand new passphrase";

async fn request(env: &common::Env, email: &str) -> common::Reply {
    call(
        &env.app,
        "POST",
        "/v1/auth/password-reset/request",
        None,
        Some(json!({"email": email})),
    )
    .await
}

async fn confirm(env: &common::Env, token: &str, password: &str) -> common::Reply {
    call(
        &env.app,
        "POST",
        "/v1/auth/password-reset/confirm",
        None,
        Some(json!({"token": token, "password": password})),
    )
    .await
}

/// The reset mails the server has got so far (people are made by invitation, which is mailed too).
fn reset_mails(smtp: &FakeSmtp) -> Vec<crate::common::smtp::Received> {
    smtp.received()
        .into_iter()
        .filter(|mail| mail.text().contains("/reset-password"))
        .collect()
}

/// Waits (the mail is sent by a background task) until `smtp` holds `count` reset mails.
async fn mails(smtp: &FakeSmtp, count: usize) -> Vec<crate::common::smtp::Received> {
    for _ in 0..100 {
        if reset_mails(smtp).len() >= count {
            return reset_mails(smtp);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("expected {count} mails, got {}", reset_mails(smtp).len());
}

/// The token in the mailed link: it is in the fragment, after `#`.
fn token_of(mail: &crate::common::smtp::Received) -> String {
    let text = mail.text();
    let link = text
        .split_whitespace()
        .find(|word| word.starts_with("https://penfii.example.com/reset-password"))
        .unwrap_or_else(|| panic!("no reset link in {text}"));
    let (_, token) = link.split_once("#").expect("the token is in the fragment");
    assert!(
        !link.contains('?'),
        "the token must not be in a query: {link}"
    );
    token.to_string()
}

async fn with_mail(pool: PgPool) -> (common::Env, FakeSmtp) {
    let smtp = FakeSmtp::start(Mode::Accept).await;
    let env = common::Env::with_mailer(pool, smtp.mailer(Duration::from_secs(5))).await;
    (env, smtp)
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_mailed_link_sets_a_new_password_once_and_signs_the_account_out_everywhere(pool: PgPool) {
    let (env, smtp) = with_mail(pool).await;
    let member = env.person("mina", "MEMBER").await;
    let other_device = login(&env.app, &member.email, PASSWORD).await.1.unwrap();
    let info = call(&env.app, "GET", "/v1/server-info", None, None).await;
    assert!(
        info.body["features"]
            .as_array()
            .unwrap()
            .contains(&json!("password-reset"))
    );

    let asked = request(&env, "Mina@Penfii.test").await;
    assert_eq!(asked.status, StatusCode::ACCEPTED);
    let mails = mails(&smtp, 1).await;
    assert_eq!(mails[0].to, "mina@penfii.test");
    // Korean by default; the link starts with the public origin and carries the token in the fragment.
    assert!(mails[0].text().contains("비밀번호"));
    let token = token_of(&mails[0]);
    assert!(token.len() >= 32);

    // The token is stored only as a hash.
    let stored: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.password_reset_tokens WHERE encode(token_hash, 'escape') = $1",
    )
    .bind(&token)
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(stored, 0);

    // A password that breaks the rule is refused and the link stays usable.
    let weak = confirm(&env, &token, "short").await;
    assert_eq!(weak.status, StatusCode::BAD_REQUEST, "{}", weak.body);
    let done = confirm(&env, &token, NEW_PASSWORD).await;
    assert_eq!(done.status, StatusCode::NO_CONTENT, "{}", done.body);

    // The new password works, the old one does not, and the sessions that existed are gone.
    assert_eq!(
        login(&env.app, &member.email, NEW_PASSWORD).await.0,
        StatusCode::OK
    );
    assert_eq!(
        login(&env.app, &member.email, PASSWORD).await.0,
        StatusCode::UNAUTHORIZED
    );
    for cookie in [&member.cookie, &other_device] {
        let me = call(&env.app, "GET", "/v1/me", Some(cookie), None).await;
        assert_eq!(me.status, StatusCode::UNAUTHORIZED);
    }

    // The link works once.
    let again = confirm(&env, &token, "yet another passphrase here").await;
    assert_eq!(again.status, StatusCode::CONFLICT);
    assert_eq!(again.body["code"], "reset_token_invalid");
    assert_eq!(
        login(&env.app, &member.email, NEW_PASSWORD).await.0,
        StatusCode::OK
    );

    // The audit log knows the account and the way, never the token or a password.
    let audit: Vec<(String, Value)> = sqlx::query_as(
        "SELECT action_code, data FROM penfii.audit_entries WHERE action_code LIKE 'user.password_reset%' ORDER BY occurred_at, id",
    )
    .fetch_all(&env.pool)
    .await
    .unwrap();
    assert_eq!(
        audit
            .iter()
            .map(|(code, _)| code.as_str())
            .collect::<Vec<_>>(),
        ["user.password_reset_requested", "user.password_reset"]
    );
    let text = format!("{audit:?}");
    assert!(
        !text.contains(&token) && !text.contains(NEW_PASSWORD) && !text.contains("penfii.test")
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn every_kind_of_account_gets_the_same_answer_and_only_a_real_one_gets_mail(pool: PgPool) {
    let (env, smtp) = with_mail(pool).await;
    let real = env.person("real", "MEMBER").await;
    let disabled = env.person("disabled", "MEMBER").await;
    let external = env.person("external", "MEMBER").await;
    sqlx::query("UPDATE penfii.users SET disabled_at = now() WHERE id = $1")
        .bind(disabled.id)
        .execute(&env.pool)
        .await
        .unwrap();
    // An account that only signs in through a provider has no password to reset.
    sqlx::query("DELETE FROM penfii.password_credentials WHERE user_id = $1")
        .bind(external.id)
        .execute(&env.pool)
        .await
        .unwrap();

    let mut answers = Vec::new();
    for email in [
        real.email.as_str(),
        "nobody@penfii.test",
        disabled.email.as_str(),
        external.email.as_str(),
    ] {
        let reply = request(&env, email).await;
        answers.push((
            reply.status,
            reply.bytes.clone(),
            reply.header("content-type").map(str::to_string),
        ));
    }
    assert!(
        answers.iter().all(|answer| answer == &answers[0]),
        "{answers:?}"
    );
    assert_eq!(answers[0].0, StatusCode::ACCEPTED);

    let sent = mails(&smtp, 1).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(
        reset_mails(&smtp).len(),
        1,
        "only the real account is mailed"
    );
    assert_eq!(sent[0].to, real.email);
    let holders: Vec<String> = sqlx::query_scalar(
        "SELECT u.email_key FROM penfii.password_reset_tokens t JOIN penfii.users u ON u.id = t.user_id",
    )
    .fetch_all(&env.pool)
    .await
    .unwrap();
    assert_eq!(holders, std::slice::from_ref(&real.email));
    // Nothing in the audit log names an address that has no account.
    let audit: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.audit_entries WHERE data::text LIKE '%nobody%'",
    )
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(audit, 0);

    // A malformed address is refused the same way whatever accounts exist.
    assert_eq!(
        request(&env, "not an address").await.status,
        StatusCode::BAD_REQUEST
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn the_answer_does_not_wait_for_the_mail_server(pool: PgPool) {
    let smtp = FakeSmtp::start(Mode::Hang).await;
    let env = common::Env::with_mailer(pool, smtp.mailer(Duration::from_secs(3))).await;
    let member = env.person("mina", "MEMBER").await;
    // The account exists and the server never answers: a synchronous mail would take 3 seconds, which
    // would tell an existing account from a missing one.
    let started = Instant::now();
    assert_eq!(
        request(&env, &member.email).await.status,
        StatusCode::ACCEPTED
    );
    let existing = started.elapsed();
    let started = Instant::now();
    assert_eq!(
        request(&env, "nobody@penfii.test").await.status,
        StatusCode::ACCEPTED
    );
    let missing = started.elapsed();
    assert!(existing < Duration::from_millis(1500), "took {existing:?}");
    assert!(missing < Duration::from_millis(1500), "took {missing:?}");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_token_ends_with_its_time_a_newer_request_or_the_account(pool: PgPool) {
    let (env, smtp) = with_mail(pool).await;
    let member = env.person("mina", "MEMBER").await;

    // Expired.
    request(&env, &member.email).await;
    let first = token_of(&mails(&smtp, 1).await[0]);
    sqlx::query("UPDATE penfii.password_reset_tokens SET expires_at = now() - interval '1 second'")
        .execute(&env.pool)
        .await
        .unwrap();
    let late = confirm(&env, &first, NEW_PASSWORD).await;
    assert_eq!(
        (late.status, late.body["code"].as_str()),
        (StatusCode::CONFLICT, Some("reset_token_invalid"))
    );

    // The lifetime is 30 minutes.
    request(&env, &member.email).await;
    let second = token_of(&mails(&smtp, 2).await[1]);
    let minutes: f64 = sqlx::query_scalar(
        "SELECT (extract(epoch FROM expires_at - created_at) / 60)::float8 FROM penfii.password_reset_tokens",
    )
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert!((29.9..=30.1).contains(&minutes), "{minutes}");

    // A newer request replaces it.
    request(&env, &member.email).await;
    let third = token_of(&mails(&smtp, 3).await[2]);
    assert_ne!(second, third);
    assert_eq!(
        confirm(&env, &second, NEW_PASSWORD).await.status,
        StatusCode::CONFLICT
    );
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM penfii.password_reset_tokens")
        .fetch_one(&env.pool)
        .await
        .unwrap();
    assert_eq!(rows, 1);

    // A disabled account cannot use one.
    sqlx::query("UPDATE penfii.users SET disabled_at = now() WHERE id = $1")
        .bind(member.id)
        .execute(&env.pool)
        .await
        .unwrap();
    assert_eq!(
        confirm(&env, &third, NEW_PASSWORD).await.status,
        StatusCode::CONFLICT
    );
    // Nonsense is refused like any other bad link.
    for junk in ["", "nope", &"x".repeat(1000)] {
        assert_eq!(
            confirm(&env, junk, NEW_PASSWORD).await.status,
            StatusCode::CONFLICT
        );
    }
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn reset_requests_are_counted_on_their_own_and_a_full_counter_answers_the_same(pool: PgPool) {
    let (env, smtp) = with_mail(pool).await;
    let member = env.person("mina", "MEMBER").await;
    // The one-account-from-one-address counter lets 5 requests through in its window (the sign-in
    // default); the rest answer the same but send nothing.
    for _ in 0..8 {
        assert_eq!(
            request(&env, &member.email).await.status,
            StatusCode::ACCEPTED
        );
    }
    mails(&smtp, 5).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(reset_mails(&smtp).len(), 5);

    // The counters are not the sign-in ones: signing in still works, and failed sign-ins do not use
    // up the reset budget of another account.
    assert_eq!(
        login(&env.app, &member.email, PASSWORD).await.0,
        StatusCode::OK
    );
    let other = env.person("other", "MEMBER").await;
    for _ in 0..4 {
        assert_eq!(
            login(&env.app, &other.email, "wrong password here").await.0,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        request(&env, &other.email).await.status,
        StatusCode::ACCEPTED
    );
    mails(&smtp, 6).await;
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn without_e_mail_there_is_no_reset(pool: PgPool) {
    let env = common::Env::new(pool).await;
    let member = env.person("mina", "MEMBER").await;
    for email in [member.email.as_str(), "nobody@penfii.test"] {
        assert_eq!(request(&env, email).await.status, StatusCode::NOT_FOUND);
    }
    assert_eq!(
        confirm(&env, "anything", NEW_PASSWORD).await.status,
        StatusCode::NOT_FOUND
    );
    let info = call(&env.app, "GET", "/v1/server-info", None, None).await;
    assert!(
        !info.body["features"]
            .as_array()
            .unwrap()
            .contains(&json!("password-reset"))
    );
    // The operator's way still works.
    assert_eq!(
        login(&env.app, &member.email, PASSWORD).await.0,
        StatusCode::OK
    );
}

/// How many of `calls` (email, address) are let through when they all arrive at the same moment.
async fn admitted(pool: &PgPool, calls: Vec<(String, Option<String>)>) -> usize {
    let limits = penfii_core::ThrottleLimits::default();
    let results = futures_util::future::join_all(calls.iter().map(|(email, address)| {
        penfii_core::identity::admit_reset_request(pool, &limits, email, address.as_deref())
    }))
    .await;
    results
        .into_iter()
        .filter(|admitted| *admitted.as_ref().unwrap())
        .count()
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn requests_that_arrive_together_cannot_get_past_a_counter(pool: PgPool) {
    // `Env` migrates the database.
    let pool = common::Env::new(pool).await.pool;
    // Each of the three counters (see `ThrottleLimits`) holds when 40 requests come at once; the counter
    // under test is the only one that can stop that case, the others are far from full.
    // One account from one address: 5.
    let same: Vec<_> = (0..40)
        .map(|_| ("a@penfii.test".to_string(), Some("10.0.0.1".to_string())))
        .collect();
    assert_eq!(admitted(&pool, same).await, 5);
    // One account from anywhere (every request from another address): 20.
    let many_addresses: Vec<_> = (0..40)
        .map(|i| ("b@penfii.test".to_string(), Some(format!("10.1.0.{i}"))))
        .collect();
    assert_eq!(admitted(&pool, many_addresses).await, 20);
    // One address for any account: 30.
    let many_accounts: Vec<_> = (0..40)
        .map(|i| (format!("c{i}@penfii.test"), Some("10.2.0.1".to_string())))
        .collect();
    assert_eq!(admitted(&pool, many_accounts).await, 30);
    // A request that is turned away uses up nothing: the account that was full stays full for the
    // others only as long as it is, and a different account from a fresh address is let through.
    assert_eq!(
        admitted(
            &pool,
            vec![(
                "fresh@penfii.test".to_string(),
                Some("10.3.0.1".to_string())
            )]
        )
        .await,
        1
    );
    // Without an address (no peer) only the account counters apply.
    let anonymous: Vec<_> = (0..40)
        .map(|_| ("d@penfii.test".to_string(), None))
        .collect();
    assert_eq!(admitted(&pool, anonymous).await, 5);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn requests_made_together_leave_one_live_token_and_one_working_link(pool: PgPool) {
    let (env, smtp) = with_mail(pool).await;
    let member = env.person("mina", "MEMBER").await;
    let mailer = smtp.mailer(Duration::from_secs(5));
    // The tasks the handler starts, all at once and without the counters in front of them.
    futures_util::future::join_all(
        (0..8).map(|_| {
            penfii_core::identity::process_reset_request(&env.pool, &mailer, &member.email)
        }),
    )
    .await;
    let sent = mails(&smtp, 8).await;
    let live: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.password_reset_tokens WHERE user_id = $1 AND closed_at IS NULL",
    )
    .bind(member.id)
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(live, 1, "more than one reset token is live");
    // Of the eight mailed links exactly one still works.
    let mut worked = 0;
    for mail in &sent {
        if confirm(&env, &token_of(mail), NEW_PASSWORD).await.status == StatusCode::NO_CONTENT {
            worked += 1;
        }
    }
    assert_eq!(worked, 1);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn the_database_allows_one_live_reset_token_per_account(pool: PgPool) {
    let env = common::Env::new(pool).await;
    let member = env.person("mina", "MEMBER").await;
    let insert = |hash: u8| {
        sqlx::query(
            "INSERT INTO penfii.password_reset_tokens (id, user_id, token_hash, expires_at)
             VALUES (gen_random_uuid(), $1, $2, now() + interval '30 minutes')",
        )
        .bind(member.id)
        .bind(vec![hash; 32])
        .execute(&env.pool)
    };
    insert(1).await.unwrap();
    assert!(insert(2).await.is_err(), "a second live token was stored");
    // A used token is closed and does not count.
    sqlx::query("UPDATE penfii.password_reset_tokens SET closed_at = now()")
        .execute(&env.pool)
        .await
        .unwrap();
    insert(3).await.unwrap();
}
