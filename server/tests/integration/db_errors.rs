//! What the database says when it cannot serve a request in time, and what a client is told: the
//! SQLSTATE classification (`55P03`, `40P01`, `57014`, `25P03`, pool timeout, and the errors that are
//! not about availability), and the public error `db_unavailable` (HTTP 503) with its body.

use std::time::Duration;

use axum::http::StatusCode;
use penfii_core::CoreError;
use serde_json::json;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

use crate::common;

fn classified(error: sqlx::Error) -> CoreError {
    CoreError::from(error)
}

fn unavailable(error: &CoreError, cause: &str) {
    assert!(
        matches!(error, CoreError::DbUnavailable(found) if *found == cause),
        "expected db_unavailable({cause}), got {error:?}"
    );
}

/// A pool on the test database whose connections start with `settings` (`SET ...`).
async fn pool_with(pool: &PgPool, size: u32, settings: &'static [&'static str]) -> PgPool {
    PgPoolOptions::new()
        .max_connections(size)
        .acquire_timeout(Duration::from_secs(5))
        .after_connect(move |conn, _| {
            Box::pin(async move {
                for setting in settings {
                    sqlx::query(setting).execute(&mut *conn).await?;
                }
                Ok(())
            })
        })
        .connect_with((*pool.connect_options()).clone())
        .await
        .expect("pool")
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_lock_that_is_not_granted_in_time_is_unavailable_and_the_pool_recovers(pool: PgPool) {
    common::setup(&pool).await;
    let short = pool_with(&pool, 2, &["SET lock_timeout = '100ms'"]).await;
    let mut holder = pool.begin().await.unwrap();
    sqlx::query("LOCK TABLE penfii.foundation_state IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *holder)
        .await
        .unwrap();
    let waiting = sqlx::query("SELECT count(*) FROM penfii.foundation_state")
        .fetch_one(&short)
        .await
        .unwrap_err();
    assert_eq!(
        waiting
            .as_database_error()
            .and_then(|e| e.code())
            .as_deref(),
        Some("55P03")
    );
    unavailable(&classified(waiting), "lock_not_available");
    holder.rollback().await.unwrap();
    // The connection that was refused goes back to the pool and works.
    sqlx::query("SELECT count(*) FROM penfii.foundation_state")
        .fetch_one(&short)
        .await
        .unwrap();
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_cancelled_statement_is_an_internal_error_not_an_unavailable_database(pool: PgPool) {
    let short = pool_with(&pool, 1, &["SET statement_timeout = '50ms'"]).await;
    let error = sqlx::query("SELECT pg_sleep(2)")
        .execute(&short)
        .await
        .unwrap_err();
    assert_eq!(
        error.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("57014")
    );
    let error = classified(error);
    assert!(
        matches!(&error, CoreError::Internal(text) if text.contains("statement timeout")),
        "{error:?}"
    );
    // The pool is fine afterwards.
    sqlx::query("SELECT 1").execute(&short).await.unwrap();
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_deadlock_is_unavailable(pool: PgPool) {
    common::setup(&pool).await;
    sqlx::query("INSERT INTO penfii.foundation_state (key) VALUES ('dl-a'), ('dl-b')")
        .execute(&pool)
        .await
        .unwrap();
    let (mut one, mut two) = (pool.acquire().await.unwrap(), pool.acquire().await.unwrap());
    sqlx::query("BEGIN").execute(&mut *one).await.unwrap();
    sqlx::query("BEGIN").execute(&mut *two).await.unwrap();
    sqlx::query("SELECT 1 FROM penfii.foundation_state WHERE key = 'dl-a' FOR UPDATE")
        .execute(&mut *one)
        .await
        .unwrap();
    sqlx::query("SELECT 1 FROM penfii.foundation_state WHERE key = 'dl-b' FOR UPDATE")
        .execute(&mut *two)
        .await
        .unwrap();
    // Each wants the row the other holds; PostgreSQL ends one of them after deadlock_timeout.
    let (first, second) = tokio::join!(
        sqlx::query("SELECT 1 FROM penfii.foundation_state WHERE key = 'dl-b' FOR UPDATE")
            .execute(&mut *one),
        sqlx::query("SELECT 1 FROM penfii.foundation_state WHERE key = 'dl-a' FOR UPDATE")
            .execute(&mut *two),
    );
    let failed: Vec<sqlx::Error> = [first, second]
        .into_iter()
        .filter_map(Result::err)
        .collect();
    assert_eq!(failed.len(), 1, "exactly one side is the victim");
    assert_eq!(
        failed[0]
            .as_database_error()
            .and_then(|e| e.code())
            .as_deref(),
        Some("40P01")
    );
    unavailable(
        &classified(failed.into_iter().next().unwrap()),
        "deadlock_detected",
    );
    for conn in [&mut one, &mut two] {
        let _ = sqlx::query("ROLLBACK").execute(&mut **conn).await;
    }
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_session_ended_for_idling_in_a_transaction_is_unavailable_and_the_pool_recovers(
    pool: PgPool,
) {
    let short = pool_with(
        &pool,
        1,
        &["SET idle_in_transaction_session_timeout = '200ms'"],
    )
    .await;
    let mut tx = short.begin().await.unwrap();
    sqlx::query("SELECT 1").execute(&mut *tx).await.unwrap();
    tokio::time::sleep(Duration::from_millis(800)).await;
    let error = sqlx::query("SELECT 1").execute(&mut *tx).await.unwrap_err();
    let error = classified(error);
    assert!(
        matches!(error, CoreError::DbUnavailable(_)),
        "the ended session is an unavailable database: {error:?}"
    );
    drop(tx);
    // The pool drops the dead connection and opens another.
    sqlx::query("SELECT 1")
        .execute(&short)
        .await
        .expect("pool recovered");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn no_connection_within_the_wait_is_unavailable(pool: PgPool) {
    let tiny = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_millis(150))
        .connect_with((*pool.connect_options()).clone())
        .await
        .unwrap();
    let held = tiny.acquire().await.unwrap();
    let error = tiny.acquire().await.expect_err("no second connection");
    unavailable(&classified(error), "pool_timeout");
    drop(held);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn violations_and_programming_errors_are_not_about_availability(pool: PgPool) {
    common::setup(&pool).await;
    // unique, foreign key, syntax, missing table: each stays an internal error for its caller to decide.
    sqlx::query("INSERT INTO penfii.foundation_state (key) VALUES ('same')")
        .execute(&pool)
        .await
        .unwrap();
    let unique = sqlx::query("INSERT INTO penfii.foundation_state (key) VALUES ('same')")
        .execute(&pool)
        .await
        .unwrap_err();
    assert!(unique.as_database_error().unwrap().is_unique_violation());
    let foreign = sqlx::query(
        "INSERT INTO penfii.workspace_members (workspace_id, user_id, role, state)
         VALUES (gen_random_uuid(), gen_random_uuid(), 'MEMBER', 'ACTIVE')",
    )
    .execute(&pool)
    .await
    .unwrap_err();
    assert!(
        foreign
            .as_database_error()
            .unwrap()
            .is_foreign_key_violation()
    );
    let syntax = sqlx::query("SELEC 1").execute(&pool).await.unwrap_err();
    let missing = sqlx::query("SELECT 1 FROM penfii.no_such_table")
        .execute(&pool)
        .await
        .unwrap_err();
    for error in [unique, foreign, syntax, missing] {
        let error = classified(error);
        assert!(matches!(error, CoreError::Internal(_)), "{error:?}");
    }
}

/// A router over a pool whose sessions start with `settings`, the way the runtime role's do.
async fn app_with(pool: &PgPool, settings: &'static [&'static str]) -> axum::Router {
    penfii_http_api::router(
        pool_with(pool, 4, settings).await,
        common::limits(),
        false,
        None,
    )
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_request_the_database_could_not_serve_in_time_is_503_db_unavailable(pool: PgPool) {
    let boot = common::setup(&pool).await;
    let app = app_with(&pool, &["SET lock_timeout = '150ms'"]).await;
    let cookie = common::sign_in(&app, common::OWNER_EMAIL, common::OWNER_PASSWORD).await;

    let mut holder = pool.begin().await.unwrap();
    sqlx::query("LOCK TABLE penfii.channels IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *holder)
        .await
        .unwrap();
    let reply = common::send(
        &app,
        common::request(
            "POST",
            &format!("/v1/workspaces/{}/channels", boot.workspace_id),
            Some(&cookie),
            None,
            Some(json!({"name":"blocked","visibility":"PUBLIC","teamId":null,"projectId":null})),
        ),
    )
    .await;
    assert_eq!(
        reply.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{:?}",
        reply.body
    );
    // The usual {code, message} and nothing else: no retry hint, nothing the database said.
    let body = reply.body.clone();
    assert_eq!(
        body,
        json!({"code": "db_unavailable", "message": "the database is temporarily unavailable"})
    );
    assert!(reply.header("retry-after").is_none());
    let text = body.to_string().to_lowercase();
    for word in [
        "55p03", "lock", "penfii.", "channels", "postgres", "sqlstate", "retry",
    ] {
        assert!(!text.contains(word), "{word} in {text}");
    }
    holder.rollback().await.unwrap();

    // Once the lock is gone the same request works.
    let reply = common::send(
        &app,
        common::request(
            "POST",
            &format!("/v1/workspaces/{}/channels", boot.workspace_id),
            Some(&cookie),
            None,
            Some(json!({"name":"free","visibility":"PUBLIC","teamId":null,"projectId":null})),
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{:?}", reply.body);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_cancelled_statement_is_a_plain_500_not_db_unavailable(pool: PgPool) {
    let boot = common::setup(&pool).await;
    // Only the statement timeout is set, so the wait for the lock ends as `57014`.
    let app = app_with(&pool, &["SET statement_timeout = '150ms'"]).await;
    let cookie = common::sign_in(&app, common::OWNER_EMAIL, common::OWNER_PASSWORD).await;
    let mut holder = pool.begin().await.unwrap();
    sqlx::query("LOCK TABLE penfii.channels IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *holder)
        .await
        .unwrap();
    let reply = common::send(
        &app,
        common::request(
            "POST",
            &format!("/v1/workspaces/{}/channels", boot.workspace_id),
            Some(&cookie),
            None,
            Some(json!({"name":"slow","visibility":"PUBLIC","teamId":null,"projectId":null})),
        ),
    )
    .await;
    assert_eq!(
        reply.status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "{:?}",
        reply.body
    );
    assert_eq!(
        reply.body,
        json!({"code": "internal", "message": "internal error"})
    );
    holder.rollback().await.unwrap();
}
