//! The sign-in throttle (three counters, configurable) and the rule for a password somebody sets.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use penfii_core::identity::{self, BootstrapInput};
use penfii_core::{Bucket, CoreError, Limits, ThrottleLimits};
use serde_json::json;
use sqlx::PgPool;
use tower::ServiceExt;

use crate::common::{self, OWNER_EMAIL as OWNER, OWNER_PASSWORD, call_parts as call};

fn limits(login_throttle: ThrottleLimits) -> Limits {
    Limits {
        login_throttle,
        ..common::limits()
    }
}

async fn attempt(
    pool: &PgPool,
    limits: &Limits,
    email: &str,
    password: &str,
    address: &str,
) -> Result<(), CoreError> {
    identity::login_from(pool, limits, email, password, Some(address), None)
        .await
        .map(|_| ())
}

fn locked(result: Result<(), CoreError>) -> bool {
    matches!(result, Err(CoreError::RateLimited { .. }))
}

fn wrong(result: Result<(), CoreError>) -> bool {
    matches!(result, Err(CoreError::Unauthenticated))
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn sign_in_throttle_counters(pool: PgPool) {
    common::setup(&pool).await;
    throttle_counters(&pool).await;
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn sign_in_throttle_limits_are_configurable(pool: PgPool) {
    common::setup(&pool).await;
    custom_limits(&pool).await;
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn password_rule_applies_wherever_a_password_is_set(pool: PgPool) {
    common::setup(&pool).await;
    password_rule(&pool).await;
}

async fn throttle_counters(pool: &PgPool) {
    let limits = limits(ThrottleLimits::default());

    // 1. One address cannot lock the person out. The attacker's own e-mail+address counter fills at 5
    //    and refuses them from then on, but the owner signing in from elsewhere is untouched.
    for _ in 0..5 {
        assert!(wrong(
            attempt(pool, &limits, OWNER, "guess", "198.51.100.1").await
        ));
    }
    assert!(
        locked(attempt(pool, &limits, OWNER, OWNER_PASSWORD, "198.51.100.1").await),
        "the attacking address is locked, even for the right password"
    );
    assert!(
        attempt(pool, &limits, OWNER, OWNER_PASSWORD, "203.0.113.9")
            .await
            .is_ok(),
        "the owner, from another address, signs in"
    );

    // 2. A success clears the counters: five more wrong passwords from the owner's address are
    //    needed to lock it again.
    for _ in 0..4 {
        assert!(wrong(
            attempt(pool, &limits, OWNER, "guess", "203.0.113.9").await
        ));
    }
    assert!(
        attempt(pool, &limits, OWNER, OWNER_PASSWORD, "203.0.113.9")
            .await
            .is_ok()
    );

    // 3. A guess spread over many addresses still stops: 4 addresses x 5 failures fill the account
    //    counter (20), and then nobody gets in for that e-mail, the owner included.
    for n in 10..14 {
        let address = format!("192.0.2.{n}");
        for _ in 0..5 {
            assert!(wrong(
                attempt(pool, &limits, OWNER, "guess", &address).await
            ));
        }
    }
    assert!(
        locked(attempt(pool, &limits, OWNER, OWNER_PASSWORD, "203.0.113.77").await),
        "20 failures from anywhere lock the account counter"
    );
    // ... and an unrelated account from a fresh address is not affected.
    let other = "ghost@penfii.test";
    assert!(wrong(
        attempt(pool, &limits, other, "guess", "203.0.113.77").await
    ));

    // 4. One address, many e-mails: the address counter (30) locks that address for every account.
    sqlx::query("DELETE FROM penfii.auth_throttle")
        .execute(pool)
        .await
        .unwrap();
    for n in 0..30 {
        let email = format!("nobody{n}@penfii.test");
        assert!(wrong(
            attempt(pool, &limits, &email, "guess", "198.51.100.200").await
        ));
    }
    assert!(locked(
        attempt(pool, &limits, OWNER, OWNER_PASSWORD, "198.51.100.200").await
    ));
    assert!(
        attempt(pool, &limits, OWNER, OWNER_PASSWORD, "198.51.100.201")
            .await
            .is_ok()
    );

    // 5. An unknown e-mail answers like a known one (same code, then the same lock).
    sqlx::query("DELETE FROM penfii.auth_throttle")
        .execute(pool)
        .await
        .unwrap();
    for _ in 0..5 {
        assert!(wrong(
            attempt(pool, &limits, "unknown@penfii.test", "guess", "192.0.2.50").await
        ));
    }
    assert!(locked(
        attempt(pool, &limits, "unknown@penfii.test", "guess", "192.0.2.50").await
    ));

    // 6. The window ends: after it, the counters start again.
    sqlx::query("UPDATE penfii.auth_throttle SET locked_until = now() - interval '1 second', window_start = now() - interval '1 hour'")
        .execute(pool)
        .await
        .unwrap();
    assert!(wrong(
        attempt(pool, &limits, "unknown@penfii.test", "guess", "192.0.2.50").await
    ));
}

async fn custom_limits(pool: &PgPool) {
    sqlx::query("DELETE FROM penfii.auth_throttle")
        .execute(pool)
        .await
        .unwrap();
    let tight = limits(ThrottleLimits {
        account_address: Bucket {
            max_failures: 2,
            window_seconds: 60,
        },
        account: Bucket {
            max_failures: 3,
            window_seconds: 60,
        },
        address: Bucket {
            max_failures: 4,
            window_seconds: 60,
        },
    });
    // Two failures lock e-mail+address.
    assert!(wrong(
        attempt(pool, &tight, OWNER, "guess", "192.0.2.1").await
    ));
    assert!(wrong(
        attempt(pool, &tight, OWNER, "guess", "192.0.2.1").await
    ));
    assert!(locked(
        attempt(pool, &tight, OWNER, OWNER_PASSWORD, "192.0.2.1").await
    ));
    // A third failure from another address fills the account counter.
    assert!(wrong(
        attempt(pool, &tight, OWNER, "guess", "192.0.2.2").await
    ));
    assert!(locked(
        attempt(pool, &tight, OWNER, OWNER_PASSWORD, "192.0.2.3").await
    ));
    // The lock lasts the configured window, not 15 minutes.
    let lock_seconds: f64 = sqlx::query_scalar(
        "SELECT max(extract(epoch FROM locked_until - now()))::float8 FROM penfii.auth_throttle WHERE scope = 'login'",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert!(
        lock_seconds > 30.0 && lock_seconds <= 60.0,
        "{lock_seconds}"
    );
    // Four failures from one address lock it for other e-mails.
    for n in 0..4 {
        let email = format!("nobody{n}@penfii.test");
        assert!(wrong(
            attempt(pool, &tight, &email, "guess", "192.0.2.9").await
        ));
    }
    assert!(locked(
        attempt(pool, &tight, "nobody9@penfii.test", "guess", "192.0.2.9").await
    ));
    sqlx::query("DELETE FROM penfii.auth_throttle")
        .execute(pool)
        .await
        .unwrap();
}

fn assert_refused(result: Result<impl std::fmt::Debug, CoreError>, why: &str) {
    match result {
        Err(CoreError::Validation(_)) => {}
        other => panic!("{why}: expected a validation error, got {other:?}"),
    }
}

async fn password_rule(pool: &PgPool) {
    let app = penfii_http_api::router(pool.clone(), limits(ThrottleLimits::default()), false, None);
    let input = |email: &str, password: &str| BootstrapInput {
        email: email.into(),
        password: password.into(),
        display_name: "Dana".into(),
        workspace_name: "Dana's".into(),
        time_zone: "Asia/Seoul".into(),
    };

    // Creating an account.
    assert_refused(
        identity::create_account(pool, input("dana@penfii.test", "short-pass")).await,
        "14 characters or fewer",
    );
    assert_refused(
        identity::create_account(pool, input("dana@penfii.test", "passwordpassword")).await,
        "a common word, doubled",
    );
    assert_refused(
        identity::create_account(pool, input("dana@penfii.test", &"x1".repeat(65))).await,
        "129 characters",
    );
    assert!(
        identity::create_account(pool, input("dana@penfii.test", "purple otter 日本語 pass"))
            .await
            .is_ok(),
        "spaces and non-ASCII are allowed, nothing is required"
    );

    // The operator command.
    assert_refused(
        identity::admin_set_password(pool, OWNER, "tiny").await,
        "admin short",
    );
    assert_refused(
        identity::admin_set_password(pool, OWNER, "1234567890123456").await,
        "admin run",
    );
    identity::admin_set_password(pool, OWNER, "recovered-test-passphrase")
        .await
        .expect("admin sets a good password");

    // Changing it from the profile.
    let (status, _, cookie) = call(
        &app,
        "POST",
        "/v1/session/login",
        None,
        Some(json!({"email": OWNER, "password": "recovered-test-passphrase"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie = cookie.unwrap();
    let patch = |new_password: &str| json!({"displayName": "Owner", "email": OWNER, "newPassword": new_password});
    let (status, body, _) = call(
        &app,
        "PATCH",
        "/v1/me",
        Some(&cookie),
        Some(patch("too short")),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["code"], "validation");
    let (status, body, _) = call(
        &app,
        "PATCH",
        "/v1/me",
        Some(&cookie),
        Some(patch("owner owner owner")),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, body, _) = call(
        &app,
        "PATCH",
        "/v1/me",
        Some(&cookie),
        Some(patch("a fresh and unusual phrase")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Accepting an invitation.
    let workspace_id: String = sqlx::query_scalar("SELECT id::text FROM penfii.workspaces LIMIT 1")
        .fetch_one(pool)
        .await
        .unwrap();
    let (status, invitation, _) = call(
        &app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/invitations"),
        Some(&cookie),
        Some(json!({"email": "guest@penfii.test", "role": "MEMBER"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{invitation}");
    let accept = |password: &str| json!({"token": invitation["inviteToken"], "displayName": "Guest", "password": password});
    let (status, body, _) = call(
        &app,
        "POST",
        "/v1/invitations/accept",
        None,
        Some(accept("12345")),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, body, _) = call(
        &app,
        "POST",
        "/v1/invitations/accept",
        None,
        Some(accept("guest guest guest guest")),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, body, _) = call(
        &app,
        "POST",
        "/v1/invitations/accept",
        None,
        Some(accept("lantern orbit plum dune")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    // Signing in does not apply the rule: an account whose password was set earlier, under a weaker
    // rule (the development seed's `1234`), still signs in.
    identity_seed(pool).await;
    let (status, body, _) = call(
        &app,
        "POST",
        "/v1/session/login",
        None,
        Some(json!({"email": "alice@ourspace.dev", "password": "1234"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

async fn identity_seed(pool: &PgPool) {
    // `seed_dev` only runs in a debug build, which is what `cargo test` is.
    let seeded = penfii_core::directory::seed_dev(pool).await.expect("seed");
    assert!(seeded, "the development seed ran");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn every_response_carries_nosniff_and_a_strict_policy(pool: PgPool) {
    common::setup(&pool).await;
    let app = penfii_http_api::router(pool, limits(ThrottleLimits::default()), false, None);
    for (method, path) in [
        ("GET", "/v1/server-info"),
        ("GET", "/v1/me"),
        ("GET", "/readyz"),
        ("GET", "/nothing-here"),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let headers = response.headers();
        assert_eq!(headers["x-content-type-options"], "nosniff", "{path}");
        assert_eq!(
            headers["content-security-policy"], "default-src 'none'; frame-ancestors 'none'",
            "{path}"
        );
    }
}

/// A pool of `size` connections on the test database, with a short wait for a connection, so a
/// connection held too long shows up as a timeout instead of a slow test.
async fn small_pool(pool: &PgPool, size: u32) -> PgPool {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(size)
        .acquire_timeout(std::time::Duration::from_millis(250))
        .connect_with((*pool.connect_options()).clone())
        .await
        .expect("small pool")
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn sign_in_holds_no_connection_while_the_password_is_hashed(pool: PgPool) {
    common::setup(&pool).await;
    let small = small_pool(&pool, 1).await;
    let limits = limits(ThrottleLimits::default());
    let login = tokio::spawn({
        let (small, limits) = (small.clone(), limits.clone());
        async move {
            identity::login_from(
                &small,
                &limits,
                OWNER,
                "not the password at all",
                Some("203.0.113.9"),
                None,
            )
            .await
            .map(|_| ())
        }
    });
    // The pool has one connection. While the password is being hashed (hundreds of milliseconds in a
    // test build) it has to be free; a connection or transaction held across the hash would make
    // every probe wait out its 250 ms and fail.
    let mut probes = 0;
    while !login.is_finished() {
        let mut conn = small
            .acquire()
            .await
            .expect("the only connection is free while the password is hashed");
        sqlx::query("SELECT 1").execute(&mut *conn).await.unwrap();
        drop(conn);
        probes += 1;
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    assert!(probes > 0);
    assert!(wrong(login.await.unwrap()));
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn concurrent_sign_ins_do_not_starve_a_small_pool(pool: PgPool) {
    common::setup(&pool).await;
    let small = small_pool(&pool, 2).await;
    // No throttle gets in the way: every attempt is checked (and hashed), whoever it is.
    let limits = limits(ThrottleLimits {
        account: Bucket {
            max_failures: 1000,
            window_seconds: 60,
        },
        account_address: Bucket {
            max_failures: 1000,
            window_seconds: 60,
        },
        address: Bucket {
            max_failures: 1000,
            window_seconds: 60,
        },
    });
    let mut attempts = Vec::new();
    for n in 0..12 {
        let (small, limits) = (small.clone(), limits.clone());
        attempts.push(tokio::spawn(async move {
            // Wrong password, existing and missing accounts, and the right password.
            let (email, password) = match n % 3 {
                0 => (OWNER.to_string(), "wrong password".to_string()),
                1 => (
                    format!("nobody{n}@penfii.test"),
                    "wrong password".to_string(),
                ),
                _ => (OWNER.to_string(), OWNER_PASSWORD.to_string()),
            };
            let address = format!("203.0.113.{n}");
            let result =
                identity::login_from(&small, &limits, &email, &password, Some(&address), None)
                    .await
                    .map(|_| ());
            (n % 3, result)
        }));
    }
    for attempt in attempts {
        let (kind, result) = attempt.await.unwrap();
        match kind {
            2 => assert!(result.is_ok(), "{result:?}"),
            _ => assert!(wrong(result)),
        }
    }
}
