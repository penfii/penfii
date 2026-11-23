//! FEA-145: the audit entries of sign-in and session events hold no secret and cannot grow without
//! bound from failures of strangers. Feature tests of CSRF, cookies, sockets and fetchers live with
//! their features (sessions.rs, external_login.rs, mcp_oauth.rs, collab.rs ...).

use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;

use crate::common::{self, call_parts as call, login};

async fn count(pool: &PgPool, action: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM penfii.audit_entries WHERE action_code = $1")
        .bind(action)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn sign_in_and_session_events_are_audited_without_secrets(pool: PgPool) {
    common::setup(&pool).await;
    let app = penfii_http_api::router(pool.clone(), common::limits(), false, None);

    let (status, cookie) = login(&app, "owner@penfii.test", "owner-test-passphrase").await;
    assert_eq!(status, StatusCode::OK);
    let cookie = cookie.unwrap();
    assert_eq!(count(&pool, "user.login").await, 1);

    let (_, other) = login(&app, "owner@penfii.test", "owner-test-passphrase").await;
    let _ = other.unwrap();
    let (status, body, _) = call(&app, "DELETE", "/v1/me/sessions", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(count(&pool, "user.sessions_revoked").await, 1);
    let data: serde_json::Value = sqlx::query_scalar(
        "SELECT data FROM penfii.audit_entries WHERE action_code = 'user.sessions_revoked'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(data, json!({"count": 1}));

    // A password changed from the profile is recorded, without either password.
    let (status, body, _) = call(
        &app,
        "PATCH",
        "/v1/me",
        Some(&cookie),
        Some(json!({"displayName": "Owner", "email": "owner@penfii.test",
                    "currentPassword": "owner-test-passphrase",
                    "newPassword": "a changed test passphrase"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(count(&pool, "user.password_changed").await, 1);

    // Nothing the audit log holds names a password, a token or the session cookie.
    let dump: String = sqlx::query_scalar(
        "SELECT coalesce(string_agg(action_code || data::text, ' '), '') FROM penfii.audit_entries",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    for secret in [
        "owner-test-passphrase",
        "a changed test passphrase",
        cookie.as_str(),
    ] {
        assert!(!dump.contains(secret));
    }
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn failed_sign_ins_of_strangers_leave_no_audit_rows_and_a_lock_leaves_one(pool: PgPool) {
    common::setup(&pool).await;
    let app = penfii_http_api::router(pool.clone(), common::limits(), false, None);
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM penfii.audit_entries")
        .fetch_one(&pool)
        .await
        .unwrap();

    // Unknown addresses: failures, locks and refusals, and not one audit row.
    for i in 0..40 {
        let (status, _) = login(&app, &format!("ghost{}@penfii.test", i % 4), "nope").await;
        assert!(
            status == StatusCode::UNAUTHORIZED || status == StatusCode::TOO_MANY_REQUESTS,
            "{status}"
        );
    }
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM penfii.audit_entries")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(before, after);

    // A real account: many wrong passwords write one entry, the one that set the lock.
    for _ in 0..20 {
        let _ = login(&app, "owner@penfii.test", "nope").await;
    }
    assert_eq!(count(&pool, "user.login_locked").await, 1);
    assert_eq!(count(&pool, "user.login").await, 0);
}
