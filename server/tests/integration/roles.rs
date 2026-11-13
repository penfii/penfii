//! The two database roles: the owner (migrations) and the runtime role (core-api), and the check
//! that the schema is current. `admin-cli db-provision` / `admin-cli migrate` use the same code.

use penfii_core::provision::{Provision, provision};
use penfii_core::{apply_migrations, connect, protect_history, require_current_schema};
use sqlx::{Connection, PgPool, Row};

use crate::common::{self, provision_roles, role_names, url_as};

async fn pool_as(pool: &PgPool, role: &str, password: &str) -> PgPool {
    connect(&url_as(pool, role, password))
        .await
        .unwrap_or_else(|error| panic!("connect as {role}: {error}"))
}

/// The error code of a failed statement, when it is a database error.
fn code(result: Result<impl std::fmt::Debug, sqlx::Error>) -> String {
    match result {
        Ok(value) => panic!("expected a database error, got {value:?}"),
        Err(error) => error
            .as_database_error()
            .and_then(|e| e.code().map(|c| c.to_string()))
            .unwrap_or_else(|| format!("not a database error: {error}")),
    }
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_non_superuser_owner_applies_every_migration_and_the_runtime_role_cannot_change_the_schema(
    pool: PgPool,
) {
    let _turn = common::MIGRATING.acquire().await.unwrap();
    let (owner, runtime) = role_names();
    let done = provision_roles(&pool, &owner, &runtime).await;
    assert!(done.owner_created && done.runtime_created);

    // Neither role is a superuser, may create databases or roles.
    for role in [&owner, &runtime] {
        let (superuser, createdb, createrole): (bool, bool, bool) = sqlx::query_as(
            "SELECT rolsuper, rolcreatedb, rolcreaterole FROM pg_roles WHERE rolname = $1",
        )
        .bind(role)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(!superuser && !createdb && !createrole, "{role}");
    }

    // Migration 0001..0008, applied by the owner and nobody else. Nothing needs a superuser.
    let owner_pool = pool_as(&pool, &owner, "owner-pass").await;
    apply_migrations(&owner_pool)
        .await
        .expect("migrations as the owner");
    protect_history(&owner_pool).await.unwrap();
    let applied: i64 =
        sqlx::query_scalar("SELECT count(*) FROM penfii._sqlx_migrations WHERE success")
            .fetch_one(&owner_pool)
            .await
            .unwrap();
    assert_eq!(applied, 8);
    // Again: nothing to do.
    apply_migrations(&owner_pool).await.unwrap();

    // The runtime role sees a current schema, reads and writes rows ...
    let app = pool_as(&pool, &runtime, "runtime-pass").await;
    require_current_schema(&app).await.expect("current schema");
    sqlx::query("INSERT INTO penfii.foundation_state (key) VALUES ('runtime-wrote-this')")
        .execute(&app)
        .await
        .unwrap();
    let rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.foundation_state WHERE key = 'runtime-wrote-this'",
    )
    .fetch_one(&app)
    .await
    .unwrap();
    assert_eq!(rows, 1);
    // ... and everything that changes a schema or a role is refused (42501 insufficient_privilege).
    assert_eq!(
        code(
            sqlx::query("CREATE TABLE penfii.sneaky (id int)")
                .execute(&app)
                .await
        ),
        "42501"
    );
    assert_eq!(
        code(
            sqlx::query("ALTER TABLE penfii.users ADD COLUMN sneaky int")
                .execute(&app)
                .await
        ),
        "42501"
    );
    assert_eq!(
        code(sqlx::query("DROP TABLE penfii.users").execute(&app).await),
        "42501"
    );
    assert_eq!(
        code(sqlx::query("CREATE SCHEMA sneaky").execute(&app).await),
        "42501"
    );
    assert_eq!(
        code(sqlx::query("CREATE ROLE sneaky").execute(&app).await),
        "42501"
    );
    assert_eq!(
        code(sqlx::query("CREATE DATABASE sneaky").execute(&app).await),
        "42501"
    );
    // The history is read, never written.
    sqlx::query("SELECT version FROM penfii._sqlx_migrations")
        .fetch_all(&app)
        .await
        .unwrap();
    assert_eq!(
        code(
            sqlx::query("UPDATE penfii._sqlx_migrations SET checksum = ''::bytea")
                .execute(&app)
                .await
        ),
        "42501"
    );
    assert_eq!(
        code(
            sqlx::query("DELETE FROM penfii._sqlx_migrations")
                .execute(&app)
                .await
        ),
        "42501"
    );
    // It owns nothing in the schema.
    let owned_by_runtime: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
         WHERE n.nspname = 'penfii' AND pg_get_userbyid(c.relowner) = $1",
    )
    .bind(&runtime)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(owned_by_runtime, 0);

    // The timeouts belong to the runtime role: its sessions have them, the owner's do not.
    let show = |pool: PgPool, name: &'static str| async move {
        sqlx::query(&format!("SHOW {name}"))
            .fetch_one(&pool)
            .await
            .unwrap()
            .get::<String, _>(0)
    };
    assert_eq!(show(app.clone(), "lock_timeout").await, "5s");
    assert_eq!(show(app.clone(), "statement_timeout").await, "30s");
    assert_eq!(
        show(app.clone(), "idle_in_transaction_session_timeout").await,
        "1min"
    );
    assert_eq!(show(owner_pool.clone(), "lock_timeout").await, "0");
    assert_eq!(show(owner_pool.clone(), "statement_timeout").await, "0");
    assert_eq!(
        show(owner_pool.clone(), "idle_in_transaction_session_timeout").await,
        "0"
    );

    // A table a later migration makes is usable by the runtime role at once (default privileges),
    // and the history stays protected after another run.
    sqlx::query("CREATE TABLE penfii.made_later (id int PRIMARY KEY, note text)")
        .execute(&owner_pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO penfii.made_later VALUES (1, 'x')")
        .execute(&app)
        .await
        .unwrap();
    protect_history(&owner_pool).await.unwrap();
    assert_eq!(
        code(
            sqlx::query("DELETE FROM penfii._sqlx_migrations")
                .execute(&app)
                .await
        ),
        "42501"
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn an_installation_that_exists_keeps_its_owner_and_gets_a_runtime_role(pool: PgPool) {
    // The database as an installation from before the split has it: migrated by one role (here the
    // superuser of the test server), which owns every object.
    common::setup(&pool).await;
    let (_, runtime) = role_names();
    let mut conn = sqlx::PgConnection::connect(&common::database_url(&pool))
        .await
        .unwrap();
    let existing_owner: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&mut conn)
        .await
        .unwrap();
    let done = provision(
        &mut conn,
        &Provision {
            owner_role: existing_owner.clone(),
            owner_password: Some("never-used".into()),
            runtime_role: runtime.clone(),
            runtime_password: Some("runtime-pass".into()),
        },
    )
    .await
    .unwrap();
    assert!(done.owner_kept_as_is && !done.owner_created && done.runtime_created);
    // Ownership was not moved, and the owner role was not changed.
    let other_owners: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
         WHERE n.nspname = 'penfii' AND pg_get_userbyid(c.relowner) <> $1 AND c.relkind = 'r'",
    )
    .bind(&existing_owner)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(other_owners, 0);
    let superuser: bool = sqlx::query_scalar("SELECT rolsuper FROM pg_roles WHERE rolname = $1")
        .bind(&existing_owner)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(superuser, "the existing owner is not altered");

    // The runtime role works on the existing tables, cannot write the history, and a table the
    // existing owner makes later is usable at once.
    let app = pool_as(&pool, &runtime, "runtime-pass").await;
    require_current_schema(&app).await.unwrap();
    let users: i64 = sqlx::query_scalar("SELECT count(*) FROM penfii.users")
        .fetch_one(&app)
        .await
        .unwrap();
    assert!(users >= 1);
    assert_eq!(
        code(
            sqlx::query("DELETE FROM penfii._sqlx_migrations")
                .execute(&app)
                .await
        ),
        "42501"
    );
    assert_eq!(
        code(
            sqlx::query("CREATE TABLE penfii.sneaky (id int)")
                .execute(&app)
                .await
        ),
        "42501"
    );
    sqlx::query("CREATE TABLE penfii.made_later (id int PRIMARY KEY)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO penfii.made_later VALUES (1)")
        .execute(&app)
        .await
        .unwrap();

    // Provisioning again (a restored database, a password change) is harmless and keeps the grants.
    let again = provision(
        &mut conn,
        &Provision {
            owner_role: existing_owner,
            owner_password: None,
            runtime_role: runtime.clone(),
            runtime_password: Some("changed".into()),
        },
    )
    .await
    .unwrap();
    assert!(!again.runtime_created);
    let app = pool_as(&pool, &runtime, "changed").await;
    sqlx::query("SELECT 1 FROM penfii.users LIMIT 1")
        .fetch_all(&app)
        .await
        .unwrap();
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn provisioning_refuses_names_that_are_not_plain_identifiers(pool: PgPool) {
    let mut conn = sqlx::PgConnection::connect(&common::database_url(&pool))
        .await
        .unwrap();
    for (owner, runtime) in [
        ("Owner", "runtime_ok"),
        ("owner_ok", "x\"; DROP SCHEMA penfii; --"),
        ("same_role", "same_role"),
        ("pg_owner", "runtime_ok"),
    ] {
        let result = provision(
            &mut conn,
            &Provision {
                owner_role: owner.into(),
                owner_password: None,
                runtime_role: runtime.into(),
                runtime_password: None,
            },
        )
        .await;
        assert!(result.is_err(), "{owner} / {runtime}");
    }
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn the_schema_check_says_what_is_wrong(pool: PgPool) {
    // No history at all.
    let error = require_current_schema(&pool).await.unwrap_err();
    assert!(
        error.contains("has not been migrated") && error.contains("admin-cli migrate"),
        "{error}"
    );

    let _turn = common::MIGRATING.acquire().await.unwrap();
    apply_migrations(&pool).await.unwrap();
    require_current_schema(&pool).await.unwrap();

    // Behind: the history lacks the last migration (its row is kept aside and put back).
    sqlx::query("CREATE TABLE penfii.history_aside AS SELECT * FROM penfii._sqlx_migrations WHERE version = (SELECT max(version) FROM penfii._sqlx_migrations)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM penfii._sqlx_migrations WHERE version IN (SELECT version FROM penfii.history_aside)")
        .execute(&pool)
        .await
        .unwrap();
    let error = require_current_schema(&pool).await.unwrap_err();
    assert!(
        error.contains("behind this build") && error.contains("admin-cli migrate"),
        "{error}"
    );
    sqlx::query("INSERT INTO penfii._sqlx_migrations SELECT * FROM penfii.history_aside")
        .execute(&pool)
        .await
        .unwrap();
    require_current_schema(&pool).await.unwrap();

    // Changed after it was applied.
    sqlx::query("UPDATE penfii._sqlx_migrations SET checksum = '\\x00'::bytea WHERE version = 1")
        .execute(&pool)
        .await
        .unwrap();
    let error = require_current_schema(&pool).await.unwrap_err();
    assert!(
        error.contains("was changed after it was applied"),
        "{error}"
    );

    // Recorded as failed.
    sqlx::query("UPDATE penfii._sqlx_migrations SET success = false WHERE version = 1")
        .execute(&pool)
        .await
        .unwrap();
    let error = require_current_schema(&pool).await.unwrap_err();
    assert!(error.contains("recorded as failed"), "{error}");
}
