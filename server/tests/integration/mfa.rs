//! Optional TOTP two-step sign-in with recovery codes (FEA-150): setting it up, signing in with it, the
//! single use of every code, the counters, turning it off, recovery codes, what each action ends, and the
//! operator's recovery.

use axum::http::StatusCode;
use penfii_core::identity;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::common::{self, Person, call, totp_code};

const WRONG_PASSWORD: &str = "not the passphrase";

async fn env(pool: PgPool) -> (common::Env, Person) {
    let env = common::Env::new(pool).await;
    let member = env.person("mina", "MEMBER").await;
    (env, member)
}

async fn post(env: &common::Env, who: &str, path: &str, body: Value) -> common::Reply {
    call(&env.app, "POST", path, Some(who), Some(body)).await
}

async fn login_with(
    env: &common::Env,
    who: &Person,
    password: &str,
    code: Option<&str>,
) -> common::Reply {
    let mut body = json!({"email": who.email, "password": password});
    if let Some(code) = code {
        body["code"] = json!(code);
    }
    call(&env.app, "POST", "/v1/session/login", None, Some(body)).await
}

/// A six-digit string that is none of the codes the server accepts right now.
fn wrong_code(secret: &str) -> String {
    let good: Vec<String> = (-2..=2).map(|offset| totp_code(secret, offset)).collect();
    (0..1_000_000)
        .map(|n| format!("{n:06}"))
        .find(|candidate| !good.contains(candidate))
        .unwrap()
}

/// Lets the current code be used again (a test that is not about replay does this between uses).
async fn forget_used_steps(env: &common::Env) {
    sqlx::query("UPDATE penfii.mfa_totp SET last_step = 0")
        .execute(&env.pool)
        .await
        .unwrap();
}

async fn clear_counters(env: &common::Env) {
    sqlx::query("DELETE FROM penfii.auth_throttle")
        .execute(&env.pool)
        .await
        .unwrap();
}

/// Sets MFA up for `who` through the API: the secret and the recovery codes.
async fn turn_on(env: &common::Env, who: &Person) -> (String, Vec<String>) {
    let started = post(env, &who.cookie, "/v1/me/mfa/setup", json!(null)).await;
    assert_eq!(started.status, StatusCode::OK, "{}", started.body);
    let secret = started.body["secret"].as_str().unwrap().to_string();
    let enabled = post(
        env,
        &who.cookie,
        "/v1/me/mfa/enable",
        json!({"code": totp_code(&secret, 0)}),
    )
    .await;
    assert_eq!(enabled.status, StatusCode::OK, "{}", enabled.body);
    let codes = enabled.body["recoveryCodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|code| code.as_str().unwrap().to_string())
        .collect();
    (secret, codes)
}

async fn status(env: &common::Env, who: &Person) -> Value {
    let reply = call(&env.app, "GET", "/v1/me/mfa", Some(&who.cookie), None).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    reply.body
}

async fn alive(env: &common::Env, cookie: &str) -> bool {
    call(&env.app, "GET", "/v1/me", Some(cookie), None)
        .await
        .status
        == StatusCode::OK
}

async fn grant(env: &common::Env, user: Uuid) -> Uuid {
    let client = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO penfii.oauth_clients (id, client_id, kind, client_name, redirect_uris)
         VALUES ($1, $2, 'preregistered', 'App', '[]')",
    )
    .bind(client)
    .bind(client.to_string())
    .execute(&env.pool)
    .await
    .unwrap();
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO penfii.oauth_grants (id, user_id, client_pk) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(user)
        .bind(client)
        .execute(&env.pool)
        .await
        .unwrap();
    id
}

async fn grant_revoked(env: &common::Env, id: Uuid) -> bool {
    sqlx::query_scalar("SELECT revoked_at IS NOT NULL FROM penfii.oauth_grants WHERE id = $1")
        .bind(id)
        .fetch_one(&env.pool)
        .await
        .unwrap()
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn setting_it_up_needs_a_recent_proof_and_a_code_of_the_new_secret(pool: PgPool) {
    let (env, mina) = env(pool).await;
    assert_eq!(status(&env, &mina).await["enabled"], false);

    // A stale session cannot start the setup; the answer names the ways to prove oneself.
    sqlx::query("UPDATE penfii.sessions SET last_reauth_at = now() - interval '1 hour'")
        .execute(&env.pool)
        .await
        .unwrap();
    let refused = post(&env, &mina.cookie, "/v1/me/mfa/setup", json!(null)).await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    assert_eq!(refused.body["code"], "reauth_required");
    let again = post(
        &env,
        &mina.cookie,
        "/v1/me/mfa/enable",
        json!({"code": "123456"}),
    )
    .await;
    assert_eq!(again.status, StatusCode::FORBIDDEN);
    // The proof stays the password (no code is asked for it).
    let proved = post(
        &env,
        &mina.cookie,
        "/v1/me/reauth/password",
        json!({"password": common::MEMBER_PASSWORD}),
    )
    .await;
    assert_eq!(proved.status, StatusCode::NO_CONTENT);

    // Confirming before starting has nothing to confirm.
    let early = post(
        &env,
        &mina.cookie,
        "/v1/me/mfa/enable",
        json!({"code": "123456"}),
    )
    .await;
    assert_eq!(early.status, StatusCode::CONFLICT, "{}", early.body);
    assert_eq!(early.body["code"], "mfa_setup_not_started");

    let started = post(&env, &mina.cookie, "/v1/me/mfa/setup", json!(null)).await;
    assert_eq!(started.status, StatusCode::OK);
    let secret = started.body["secret"].as_str().unwrap().to_string();
    let uri = started.body["otpauthUri"].as_str().unwrap();
    assert!(
        uri.starts_with("otpauth://totp/Penfii%3Amina%40penfii.test?secret="),
        "{uri}"
    );
    assert!(uri.contains("&digits=6&period=30"));
    assert_eq!(status(&env, &mina).await["setupPending"], true);
    assert_eq!(status(&env, &mina).await["enabled"], false);

    // A setup nobody confirmed protects nothing: the password alone still signs in, and a device that
    // signs in meanwhile is not ended by the confirmation of its own account's later enabling being absent.
    let other = login_with(&env, &mina, common::MEMBER_PASSWORD, None).await;
    assert_eq!(other.status, StatusCode::OK);
    let other_cookie = other.cookie.unwrap();

    // A wrong code does not enable it.
    let wrong = post(
        &env,
        &mina.cookie,
        "/v1/me/mfa/enable",
        json!({"code": wrong_code(&secret)}),
    )
    .await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);
    // A wrong code, not an ended session: the client keeps the person signed in.
    assert_eq!(wrong.body["code"], "mfa_code_invalid");
    assert_eq!(status(&env, &mina).await["enabled"], false);
    // Neither does something that is not a code.
    let junk = post(
        &env,
        &mina.cookie,
        "/v1/me/mfa/enable",
        json!({"code": "not-a-code"}),
    )
    .await;
    assert_eq!(junk.status, StatusCode::UNAUTHORIZED);
    // A wrong code, not an ended session: the client keeps the person signed in.
    assert_eq!(junk.body["code"], "mfa_code_invalid");

    // Starting again replaces the pending secret: the old one no longer confirms.
    let restarted = post(&env, &mina.cookie, "/v1/me/mfa/setup", json!(null)).await;
    let new_secret = restarted.body["secret"].as_str().unwrap().to_string();
    assert_ne!(new_secret, secret);
    let stale = post(
        &env,
        &mina.cookie,
        "/v1/me/mfa/enable",
        json!({"code": totp_code(&secret, 0)}),
    )
    .await;
    assert_eq!(stale.status, StatusCode::UNAUTHORIZED);
    // A wrong code, not an ended session: the client keeps the person signed in.
    assert_eq!(stale.body["code"], "mfa_code_invalid");

    let mcp = grant(&env, mina.id).await;
    let enabled = post(
        &env,
        &mina.cookie,
        "/v1/me/mfa/enable",
        json!({"code": totp_code(&new_secret, 0)}),
    )
    .await;
    assert_eq!(enabled.status, StatusCode::OK, "{}", enabled.body);
    // Recovery codes and the secret are kept by no cache.
    assert_eq!(enabled.header("cache-control"), Some("no-store"));
    assert_eq!(restarted.header("cache-control"), Some("no-store"));
    let codes = enabled.body["recoveryCodes"].as_array().unwrap();
    assert_eq!(codes.len(), 10);
    for code in codes {
        let code = code.as_str().unwrap();
        assert_eq!(code.len(), 14);
        assert_eq!(code.matches('-').count(), 2);
    }
    let after = status(&env, &mina).await;
    assert_eq!(after["enabled"], true);
    assert_eq!(after["setupPending"], false);
    assert_eq!(after["recoveryCodesRemaining"], 10);

    // This device stays, the other web session ends, the connected app stays.
    assert!(alive(&env, &mina.cookie).await);
    assert!(!alive(&env, &other_cookie).await);
    assert!(!grant_revoked(&env, mcp).await);
    // A second setup does not replace an enabled one.
    let twice = post(&env, &mina.cookie, "/v1/me/mfa/setup", json!(null)).await;
    assert_eq!(twice.status, StatusCode::CONFLICT);
    assert_eq!(twice.body["code"], "mfa_already_enabled");
    assert_eq!(status(&env, &mina).await["enabled"], true);

    // The secret is stored sealed (nonce + ciphertext + tag), under the key kept in the database.
    let (len, key_len): (i32, i32) = sqlx::query_as(
        "SELECT length(t.secret_ciphertext), length(k.key) FROM penfii.mfa_totp t, penfii.mfa_instance_key k",
    )
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!((len, key_len), (20 + 16, 32));
    let audited: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.audit_entries WHERE action_code = 'user.mfa_enabled' AND target_id = $1",
    )
    .bind(mina.id)
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(audited, 1);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_password_sign_in_needs_the_code_and_says_no_more_than_it_must(pool: PgPool) {
    let (env, mina) = env(pool).await;
    let (secret, _) = turn_on(&env, &mina).await;
    let password = common::MEMBER_PASSWORD;

    // The right password alone is `mfa_required`: no session, no cookie.
    let asked = login_with(&env, &mina, password, None).await;
    assert_eq!(asked.status, StatusCode::UNAUTHORIZED);
    assert_eq!(asked.body["code"], "mfa_required");
    assert!(asked.cookie.is_none());
    // An empty code is no code.
    let empty = login_with(&env, &mina, password, Some("  ")).await;
    assert_eq!(empty.body["code"], "mfa_required");

    // A wrong password, with or without a code, a wrong code, and an account that does not exist all
    // answer the same way: nothing tells which part was wrong.
    let good_code = totp_code(&secret, 1);
    let wrong_password = login_with(&env, &mina, WRONG_PASSWORD, None).await;
    let wrong_password_with_code = login_with(&env, &mina, WRONG_PASSWORD, Some(&good_code)).await;
    let wrong_code_reply = login_with(&env, &mina, password, Some(&wrong_code(&secret))).await;
    let ghost = Person {
        id: Uuid::nil(),
        email: "ghost@penfii.test".into(),
        cookie: String::new(),
    };
    let ghost_reply = login_with(&env, &ghost, password, Some(&good_code)).await;
    for reply in [
        &wrong_password,
        &wrong_password_with_code,
        &wrong_code_reply,
        &ghost_reply,
    ] {
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
        assert_eq!(reply.body, wrong_password.body, "{}", reply.body);
        assert!(reply.cookie.is_none());
    }
    assert_eq!(wrong_password.body["code"], "unauthenticated");

    // The right password and a current code sign in. The code is typed loosely.
    clear_counters(&env).await;
    let spaced = format!(" {} {} ", &good_code[..3], &good_code[3..]);
    let signed_in = login_with(&env, &mina, password, Some(&spaced)).await;
    assert_eq!(signed_in.status, StatusCode::OK, "{}", signed_in.body);
    let cookie = signed_in.cookie.unwrap();
    assert!(alive(&env, &cookie).await);

    // The same code is good once: the replay is refused although it is still inside the time window.
    let replay = login_with(&env, &mina, password, Some(&good_code)).await;
    assert_eq!(replay.status, StatusCode::UNAUTHORIZED);
    assert_eq!(replay.body["code"], "unauthenticated");
    // The code of the step the setup used is spent as well.
    let setup_code = login_with(&env, &mina, password, Some(&totp_code(&secret, 0))).await;
    assert_eq!(setup_code.status, StatusCode::UNAUTHORIZED);

    // The sign-in with the code is a password session like any other.
    let method: String = sqlx::query_scalar("SELECT auth_method FROM penfii.sessions WHERE id = (SELECT id FROM penfii.sessions WHERE user_id = $1 ORDER BY created_at DESC LIMIT 1)")
        .bind(mina.id)
        .fetch_one(&env.pool)
        .await
        .unwrap();
    assert_eq!(method, "password");

    // Another account is not affected by Mina's MFA.
    let other = env.person("omar", "MEMBER").await;
    assert_eq!(
        login_with(&env, &other, common::MEMBER_PASSWORD, None)
            .await
            .status,
        StatusCode::OK
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_recovery_code_works_once_and_a_spent_code_stays_spent(pool: PgPool) {
    let (env, mina) = env(pool).await;
    let (secret, codes) = turn_on(&env, &mina).await;
    let password = common::MEMBER_PASSWORD;

    let first = &codes[0];
    let typed = first.to_uppercase().replace('-', " ");
    let ok = login_with(&env, &mina, password, Some(&typed)).await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.body);
    let cookie = ok.cookie.unwrap();
    assert_eq!(status(&env, &mina).await["recoveryCodesRemaining"], 9);
    let again = login_with(&env, &mina, password, Some(first)).await;
    assert_eq!(again.status, StatusCode::UNAUTHORIZED);

    // Parallel use of one code lets exactly one request in.
    let second = codes[1].clone();
    let attempts = (0..6).map(|_| login_with(&env, &mina, password, Some(&second)));
    let replies = futures_util::future::join_all(attempts).await;
    let wins = replies
        .iter()
        .filter(|reply| reply.status == StatusCode::OK)
        .count();
    assert_eq!(
        wins,
        1,
        "{:?}",
        replies.iter().map(|r| r.status).collect::<Vec<_>>()
    );

    // A code of another account is nothing here, and neither is a made-up one.
    let other = env.person("omar", "MEMBER").await;
    let (_, other_codes) = turn_on(&env, &other).await;
    clear_counters(&env).await;
    let borrowed = login_with(&env, &mina, password, Some(&other_codes[0])).await;
    assert_eq!(borrowed.status, StatusCode::UNAUTHORIZED);
    let made_up = login_with(&env, &mina, password, Some("aaaa-bbbb-cccc")).await;
    assert_eq!(made_up.status, StatusCode::UNAUTHORIZED);

    // Using every code up leaves the authenticator, which still works; losing it too is the operator's case.
    for code in &codes[2..] {
        clear_counters(&env).await;
        let used = login_with(&env, &mina, password, Some(code)).await;
        assert_eq!(used.status, StatusCode::OK, "{}", used.body);
    }
    assert_eq!(status(&env, &mina).await["recoveryCodesRemaining"], 0);
    forget_used_steps(&env).await;
    clear_counters(&env).await;
    let by_app = login_with(&env, &mina, password, Some(&totp_code(&secret, 0))).await;
    assert_eq!(by_app.status, StatusCode::OK);
    assert!(alive(&env, &cookie).await);

    // The same authenticator code used by parallel requests lets exactly one in.
    forget_used_steps(&env).await;
    clear_counters(&env).await;
    let code = totp_code(&secret, 0);
    let replies = futures_util::future::join_all(
        (0..4).map(|_| login_with(&env, &mina, password, Some(&code))),
    )
    .await;
    let wins = replies
        .iter()
        .filter(|reply| reply.status == StatusCode::OK)
        .count();
    assert_eq!(wins, 1);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn code_attempts_are_counted_before_they_are_checked(pool: PgPool) {
    let (env, mina) = env(pool).await;
    let (secret, _) = turn_on(&env, &mina).await;
    let password = common::MEMBER_PASSWORD;
    clear_counters(&env).await;
    forget_used_steps(&env).await;

    // The right password without a code neither clears nor is charged: it is not a wrong password, so
    // asking for the code any number of times leaves the person room to send it. Wrong passwords and
    // wrong codes are counted: the fifth request of the e-mail+address counter is the last that is
    // answered, and the next one is refused for a right code too.
    for _ in 0..2 {
        let reply = login_with(&env, &mina, WRONG_PASSWORD, None).await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    }
    for _ in 0..4 {
        let asked = login_with(&env, &mina, password, None).await;
        assert_eq!(asked.body["code"], "mfa_required", "{}", asked.body);
    }
    for _ in 0..2 {
        let wrong = login_with(&env, &mina, password, Some(&wrong_code(&secret))).await;
        assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);
        // Sign-in answers a wrong code like a wrong password.
        assert_eq!(wrong.body["code"], "unauthenticated");
    }
    let fifth = login_with(&env, &mina, password, Some(&wrong_code(&secret))).await;
    assert_eq!(fifth.status, StatusCode::UNAUTHORIZED);
    let locked = login_with(&env, &mina, password, Some(&totp_code(&secret, 0))).await;
    assert_eq!(
        locked.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        locked.body
    );
    assert!(locked.body["retryAfterSeconds"].as_i64().unwrap() > 0);
    // Locked is locked for everyone who asks about the account, and for a wrong password alike.
    let also = login_with(&env, &mina, WRONG_PASSWORD, None).await;
    assert_eq!(also.status, StatusCode::TOO_MANY_REQUESTS);

    // Many guesses spread over the counters of the pair are stopped by the code counter of the account:
    // ten wrong codes in a window lock it whatever the address counters say.
    clear_counters(&env).await;
    for _ in 0..10 {
        sqlx::query("DELETE FROM penfii.auth_throttle WHERE scope <> 'mfa_login'")
            .execute(&env.pool)
            .await
            .unwrap();
        let reply = login_with(&env, &mina, password, Some(&wrong_code(&secret))).await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    }
    sqlx::query("DELETE FROM penfii.auth_throttle WHERE scope <> 'mfa_login'")
        .execute(&env.pool)
        .await
        .unwrap();
    let refused = login_with(&env, &mina, password, Some(&totp_code(&secret, 0))).await;
    assert_eq!(refused.status, StatusCode::TOO_MANY_REQUESTS);

    // The signed-in person's own attempts at turning it off have a counter of their own.
    clear_counters(&env).await;
    for _ in 0..10 {
        let reply = post(
            &env,
            &mina.cookie,
            "/v1/me/mfa/disable",
            json!({"code": wrong_code(&secret)}),
        )
        .await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    }
    let locked = post(
        &env,
        &mina.cookie,
        "/v1/me/mfa/disable",
        json!({"code": totp_code(&secret, 1)}),
    )
    .await;
    assert_eq!(locked.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(status(&env, &mina).await["enabled"], true);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn turning_it_off_and_new_recovery_codes_need_a_proof_and_a_current_code(pool: PgPool) {
    let (env, mina) = env(pool).await;
    let (secret, codes) = turn_on(&env, &mina).await;
    let mcp = grant(&env, mina.id).await;
    forget_used_steps(&env).await;
    let second = login_with(
        &env,
        &mina,
        common::MEMBER_PASSWORD,
        Some(&totp_code(&secret, 0)),
    )
    .await;
    let second_cookie = second.cookie.unwrap();

    // Without a code there is nothing to check: it is refused, not a validation of the password.
    let none = post(
        &env,
        &mina.cookie,
        "/v1/me/mfa/disable",
        json!({"code": ""}),
    )
    .await;
    assert_eq!(none.status, StatusCode::UNAUTHORIZED);
    // A wrong code, not an ended session: the client keeps the person signed in.
    assert_eq!(none.body["code"], "mfa_code_invalid");
    let wrong = post(
        &env,
        &mina.cookie,
        "/v1/me/mfa/recovery-codes",
        json!({"code": wrong_code(&secret)}),
    )
    .await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);
    // A wrong code, not an ended session: the client keeps the person signed in.
    assert_eq!(wrong.body["code"], "mfa_code_invalid");

    // A stale session is asked for the ordinary proof first, then the code.
    sqlx::query("UPDATE penfii.sessions SET last_reauth_at = now() - interval '1 hour'")
        .execute(&env.pool)
        .await
        .unwrap();
    forget_used_steps(&env).await;
    for path in ["/v1/me/mfa/disable", "/v1/me/mfa/recovery-codes"] {
        let stale = post(
            &env,
            &mina.cookie,
            path,
            json!({"code": totp_code(&secret, 0)}),
        )
        .await;
        assert_eq!(stale.status, StatusCode::FORBIDDEN, "{path}");
        assert_eq!(stale.body["code"], "reauth_required");
    }
    // The ordinary proof is the password alone, as for every other sensitive action.
    let proved = post(
        &env,
        &mina.cookie,
        "/v1/me/reauth/password",
        json!({"password": common::MEMBER_PASSWORD}),
    )
    .await;
    assert_eq!(proved.status, StatusCode::NO_CONTENT);
    let profile = call(
        &env.app,
        "PATCH",
        "/v1/me",
        Some(&mina.cookie),
        Some(json!({"displayName": "Mina 2", "email": mina.email})),
    )
    .await;
    assert_eq!(profile.status, StatusCode::OK, "{}", profile.body);

    // New recovery codes replace all the old ones; a recovery code can authorize it (and is spent).
    let fresh = post(
        &env,
        &mina.cookie,
        "/v1/me/mfa/recovery-codes",
        json!({"code": codes[0]}),
    )
    .await;
    assert_eq!(fresh.status, StatusCode::OK, "{}", fresh.body);
    assert_eq!(fresh.header("cache-control"), Some("no-store"));
    let fresh_codes: Vec<String> = fresh.body["recoveryCodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|code| code.as_str().unwrap().to_string())
        .collect();
    assert_eq!(fresh_codes.len(), 10);
    assert!(fresh_codes.iter().all(|code| !codes.contains(code)));
    assert_eq!(status(&env, &mina).await["recoveryCodesRemaining"], 10);
    assert!(
        alive(&env, &second_cookie).await,
        "new codes end no session"
    );
    clear_counters(&env).await;
    let old = login_with(&env, &mina, common::MEMBER_PASSWORD, Some(&codes[3])).await;
    assert_eq!(
        old.status,
        StatusCode::UNAUTHORIZED,
        "an old code died with the replacement"
    );
    let new = login_with(&env, &mina, common::MEMBER_PASSWORD, Some(&fresh_codes[0])).await;
    assert_eq!(new.status, StatusCode::OK);

    // Turning it off: this device stays, the other devices end, the connected app stays.
    let off = post(
        &env,
        &mina.cookie,
        "/v1/me/mfa/disable",
        json!({"code": fresh_codes[1]}),
    )
    .await;
    assert_eq!(off.status, StatusCode::NO_CONTENT, "{}", off.body);
    assert!(alive(&env, &mina.cookie).await);
    assert!(!alive(&env, &second_cookie).await);
    assert!(!grant_revoked(&env, mcp).await);
    assert_eq!(status(&env, &mina).await["enabled"], false);
    let rows: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM penfii.mfa_totp) + (SELECT count(*) FROM penfii.mfa_recovery_codes)",
    )
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(rows, 0);
    assert_eq!(
        login_with(&env, &mina, common::MEMBER_PASSWORD, None)
            .await
            .status,
        StatusCode::OK
    );
    // Nothing to turn off, nothing to replace.
    let nothing = post(
        &env,
        &mina.cookie,
        "/v1/me/mfa/disable",
        json!({"code": "123456"}),
    )
    .await;
    assert_eq!(nothing.status, StatusCode::CONFLICT);
    assert_eq!(nothing.body["code"], "mfa_not_enabled");
    let no_codes = post(
        &env,
        &mina.cookie,
        "/v1/me/mfa/recovery-codes",
        json!({"code": "123456"}),
    )
    .await;
    assert_eq!(no_codes.status, StatusCode::CONFLICT);
    // And the old recovery codes are not a way in once it is off and on again with a new secret.
    let (new_secret, _) = turn_on(&env, &mina).await;
    forget_used_steps(&env).await;
    clear_counters(&env).await;
    let ghost = login_with(&env, &mina, common::MEMBER_PASSWORD, Some(&fresh_codes[2])).await;
    assert_eq!(ghost.status, StatusCode::UNAUTHORIZED);
    let ok = login_with(
        &env,
        &mina,
        common::MEMBER_PASSWORD,
        Some(&totp_code(&new_secret, 1)),
    )
    .await;
    assert_eq!(ok.status, StatusCode::OK);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn an_account_whose_secret_cannot_be_opened_is_never_let_in_by_the_password_alone(
    pool: PgPool,
) {
    let (env, mina) = env(pool).await;
    let (secret, codes) = turn_on(&env, &mina).await;
    // The key the secret names is gone (as when the environment key was changed after the setup and the
    // database key was lost): the code cannot be checked, so the TOTP path fails closed ...
    sqlx::query("DELETE FROM penfii.mfa_instance_key")
        .execute(&env.pool)
        .await
        .unwrap();
    forget_used_steps(&env).await;
    let reply = login_with(
        &env,
        &mina,
        common::MEMBER_PASSWORD,
        Some(&totp_code(&secret, 0)),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert!(reply.cookie.is_none());
    let nothing = login_with(&env, &mina, common::MEMBER_PASSWORD, None).await;
    assert_eq!(nothing.body["code"], "mfa_required");
    // ... and a recovery code (stored as a hash, independent of the key) still gets the person in.
    let by_code = login_with(&env, &mina, common::MEMBER_PASSWORD, Some(&codes[0])).await;
    assert_eq!(by_code.status, StatusCode::OK);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn the_operator_recovers_a_locked_out_account_and_ends_everything(pool: PgPool) {
    let (env, mina) = env(pool).await;
    let (_, _) = turn_on(&env, &mina).await;
    let sibling = env.person("omar", "MEMBER").await;
    let (_, _) = turn_on(&env, &sibling).await;
    let mcp = grant(&env, mina.id).await;
    let other_mcp = grant(&env, sibling.id).await;
    // The person ran into the counters while locked out.
    for _ in 0..3 {
        let _ = login_with(&env, &mina, common::MEMBER_PASSWORD, Some("000000")).await;
    }

    assert!(matches!(
        identity::operator_reset_mfa(&env.pool, "nobody@penfii.test").await,
        Err(penfii_core::CoreError::NotFound)
    ));
    let user = identity::operator_reset_mfa(&env.pool, "MINA@penfii.test")
        .await
        .unwrap();
    assert_eq!(user, mina.id);

    assert!(!alive(&env, &mina.cookie).await, "every web session ends");
    assert!(grant_revoked(&env, mcp).await, "and every connected app");
    assert!(
        !grant_revoked(&env, other_mcp).await,
        "of that account only"
    );
    assert!(alive(&env, &sibling.cookie).await);
    let reason: Option<String> =
        sqlx::query_scalar("SELECT revoked_reason FROM penfii.oauth_grants WHERE id = $1")
            .bind(mcp)
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert_eq!(reason.as_deref(), Some("mfa_reset"));

    // The password signs in again, with no code and with no lock left from before.
    let again = login_with(&env, &mina, common::MEMBER_PASSWORD, None).await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.body);
    let fresh = again.cookie.unwrap();
    assert_eq!(
        status(
            &env,
            &Person {
                cookie: fresh.clone(),
                ..mina.clone()
            }
        )
        .await["enabled"],
        false
    );
    // The sibling's MFA is untouched, and the reset is in the audit log without a code or a secret.
    assert_eq!(status(&env, &sibling).await["enabled"], true);
    let data: Value = sqlx::query_scalar(
        "SELECT data FROM penfii.audit_entries WHERE action_code = 'user.mfa_reset' AND target_id = $1",
    )
    .bind(mina.id)
    .fetch_one(&env.pool)
    .await
    .unwrap();
    assert_eq!(data, json!({"via": "admin-cli"}));
    // They can set it up again.
    let (_, _) = turn_on(
        &env,
        &Person {
            cookie: fresh,
            ..mina
        },
    )
    .await;
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn no_other_password_path_signs_in_or_skips_the_code(pool: PgPool) {
    let (env, mina) = env(pool).await;
    let (secret, _) = turn_on(&env, &mina).await;

    // The core entry points that take a password and make a session all end in the code check.
    let limits = common::limits();
    let without = identity::login(&env.pool, &limits, &mina.email, common::MEMBER_PASSWORD).await;
    assert!(matches!(without, Err(penfii_core::CoreError::MfaRequired)));
    let from = identity::login_from(
        &env.pool,
        &limits,
        &mina.email,
        common::MEMBER_PASSWORD,
        Some("198.51.100.7"),
        None,
    )
    .await;
    assert!(matches!(from, Err(penfii_core::CoreError::MfaRequired)));
    // Accepting an invitation with the right password of an existing account does not sign it in.
    // (Mina is already in the bootstrap Workspace, so the invitation is to a second one.)
    let second = common::add_workspace(&env.pool, env.owner.id, "Second").await;
    let invitation = env
        .call(
            &env.owner,
            "POST",
            &format!("/v1/workspaces/{second}/invitations"),
            Some(json!({"email": mina.email, "role": "MEMBER"})),
        )
        .await;
    assert_eq!(
        invitation.status,
        StatusCode::CREATED,
        "{}",
        invitation.body
    );
    let accepted = call(
        &env.app,
        "POST",
        "/v1/invitations/accept",
        None,
        Some(json!({"token": invitation.body["inviteToken"], "displayName": "Mina", "password": common::MEMBER_PASSWORD})),
    )
    .await;
    assert!(accepted.cookie.is_none(), "{}", accepted.body);
    assert_ne!(accepted.status, StatusCode::CREATED, "{}", accepted.body);
    // A password reset sets a password and signs nobody in; the next sign-in still needs the code.
    // (The reset by link needs e-mail; the operator's password change is the same write.)
    identity::admin_set_password(&env.pool, &mina.email, "a brand new passphrase for mina")
        .await
        .unwrap();
    let after = login_with(&env, &mina, "a brand new passphrase for mina", None).await;
    assert_eq!(after.body["code"], "mfa_required");
    forget_used_steps(&env).await;
    clear_counters(&env).await;
    let ok = login_with(
        &env,
        &mina,
        "a brand new passphrase for mina",
        Some(&totp_code(&secret, 0)),
    )
    .await;
    assert_eq!(ok.status, StatusCode::OK);

    // A session cannot be reached with the code alone, and the MFA routes need a session.
    for path in [
        "/v1/me/mfa/setup",
        "/v1/me/mfa/enable",
        "/v1/me/mfa/disable",
        "/v1/me/mfa/recovery-codes",
    ] {
        let reply = call(
            &env.app,
            "POST",
            path,
            None,
            Some(json!({"code": "123456"})),
        )
        .await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{path}");
    }
    assert_eq!(
        call(&env.app, "GET", "/v1/me/mfa", None, None).await.status,
        StatusCode::UNAUTHORIZED
    );
}

/// FEA-182: a long session is issued only after the whole sign-in, second factor included, and it
/// is not a trusted device: the next sign-in asks for a code again.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_long_session_waits_for_the_second_factor_and_does_not_skip_it_later(pool: PgPool) {
    let (env, mina) = env(pool).await;
    let (secret, _) = turn_on(&env, &mina).await;
    clear_counters(&env).await;
    let ask = |code: Option<String>| {
        let mut body =
            json!({"email": mina.email, "password": common::MEMBER_PASSWORD, "keepSignedIn": true});
        if let Some(code) = code {
            body["code"] = json!(code);
        }
        call(&env.app, "POST", "/v1/session/login", None, Some(body))
    };
    let sessions = |env: &common::Env| {
        let pool = env.pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM penfii.sessions WHERE absolute_expires_at IS NOT NULL",
            )
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    // The password alone is refused, and nothing is issued although a long session was asked for.
    let refused = ask(None).await;
    assert_eq!(refused.body["code"], "mfa_required", "{}", refused.body);
    assert!(refused.header("set-cookie").is_none());
    assert_eq!(sessions(&env).await, 0);
    // A wrong code is refused too.
    assert_eq!(
        ask(Some("000000".into())).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(sessions(&env).await, 0);
    // The right code gives the long session, with the cookie of its absolute lifetime.
    let accepted = ask(Some(totp_code(&secret, 1))).await;
    assert_eq!(accepted.status, StatusCode::OK, "{}", accepted.body);
    assert!(
        accepted
            .header("set-cookie")
            .unwrap()
            .contains("Max-Age=2592000")
    );
    assert_eq!(sessions(&env).await, 1);
    // Not a trusted device: the same password asks for a code again.
    let again = ask(None).await;
    assert_eq!(again.body["code"], "mfa_required");
    assert_eq!(sessions(&env).await, 1);
}
