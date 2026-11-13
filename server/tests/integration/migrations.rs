//! The migration runner: one recorded history, applied once, under a lock, in a transaction.
//! Each test starts from an empty database of its own.

use std::borrow::Cow;

use crate::api::{World, call, get_as, new_project, post_as};
use axum::http::StatusCode;
use penfii_core::{apply_migrations, run_migrator};
use serde_json::json;
use sqlx::PgPool;
use sqlx::migrate::{Migration, MigrationType, Migrator};
use uuid::Uuid;

fn migrator(files: &[(i64, &str)]) -> Migrator {
    migrator_tx(files, false)
}

fn migrator_tx(files: &[(i64, &str)], no_tx: bool) -> Migrator {
    Migrator {
        migrations: Cow::Owned(
            files
                .iter()
                .map(|(version, sql)| {
                    Migration::new(
                        *version,
                        Cow::Owned(format!("m{version}")),
                        MigrationType::Simple,
                        Cow::Owned((*sql).to_owned()),
                        no_tx,
                    )
                })
                .collect(),
        ),
        ..Migrator::DEFAULT
    }
}

async fn recorded(pool: &PgPool, schema: &str) -> Vec<i64> {
    sqlx::query_scalar(&format!(
        "SELECT version FROM {schema}._sqlx_migrations WHERE success ORDER BY version"
    ))
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn exists(pool: &PgPool, table: &str) -> bool {
    sqlx::query_scalar::<_, bool>("SELECT to_regclass($1) IS NOT NULL")
        .bind(table)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn the_real_history_is_recorded_once_and_applying_again_changes_nothing(pool: PgPool) {
    let _turn = crate::common::MIGRATING.acquire().await.unwrap();
    apply_migrations(&pool).await.expect("first run");
    assert_eq!(
        recorded(&pool, "penfii").await,
        vec![1, 2, 3, 4, 5, 6, 7, 8]
    );
    let tables: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_tables WHERE schemaname = 'penfii' AND tablename <> '_sqlx_migrations'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(tables > 90, "{tables} tables");
    // A row made after the first run survives the next start (nothing is created or reset again).
    sqlx::query("INSERT INTO penfii.foundation_state (key) VALUES ('kept')")
        .execute(&pool)
        .await
        .unwrap();
    apply_migrations(&pool).await.expect("second run");
    apply_migrations(&pool).await.expect("third run");
    assert_eq!(
        recorded(&pool, "penfii").await,
        vec![1, 2, 3, 4, 5, 6, 7, 8]
    );
    let kept: i64 =
        sqlx::query_scalar("SELECT count(*) FROM penfii.foundation_state WHERE key = 'kept'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(kept, 1);
    // The runner leaves the pooled connections as it found them.
    for _ in 0..3 {
        let path: String = sqlx::query_scalar("SHOW search_path")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(!path.contains("penfii"), "{path}");
    }
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn instances_that_start_together_apply_each_migration_once(pool: PgPool) {
    let _turn = crate::common::MIGRATING.acquire().await.unwrap();
    let runs: Vec<_> = (0..6)
        .map(|_| {
            let pool = pool.clone();
            tokio::spawn(async move { apply_migrations(&pool).await })
        })
        .collect();
    for run in runs {
        run.await.unwrap().expect("every instance succeeds");
    }
    assert_eq!(
        recorded(&pool, "penfii").await,
        vec![1, 2, 3, 4, 5, 6, 7, 8]
    );
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn an_applied_file_that_was_edited_is_refused(pool: PgPool) {
    let original = migrator(&[(1, "CREATE TABLE mig_checksum.t (a int);")]);
    run_migrator(&pool, &original, "mig_checksum")
        .await
        .unwrap();
    let edited = migrator(&[(1, "CREATE TABLE mig_checksum.t (a int, b int);")]);
    let error = run_migrator(&pool, &edited, "mig_checksum")
        .await
        .expect_err("the checksum differs");
    assert!(error.to_string().contains("modified"), "{error}");
    let columns: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.columns WHERE table_schema = 'mig_checksum' AND table_name = 't'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(columns, 1);
    // The unchanged file is still fine.
    run_migrator(&pool, &original, "mig_checksum")
        .await
        .unwrap();
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_failing_file_leaves_nothing_behind_and_is_tried_again(pool: PgPool) {
    let first = "CREATE TABLE mig_fail.counter (n int); INSERT INTO mig_fail.counter VALUES (1);";
    let broken = migrator(&[
        (1, first),
        (
            2,
            "CREATE TABLE mig_fail.second (a int); INSERT INTO mig_fail.missing VALUES (1);",
        ),
    ]);
    run_migrator(&pool, &broken, "mig_fail")
        .await
        .expect_err("the second statement of file 2 fails");
    assert_eq!(recorded(&pool, "mig_fail").await, vec![1]);
    assert!(
        !exists(&pool, "mig_fail.second").await,
        "the table of the failed file was rolled back with it"
    );
    // Fixed, and started again: file 2 runs, file 1 does not run a second time.
    let fixed = migrator(&[(1, first), (2, "CREATE TABLE mig_fail.second (a int);")]);
    run_migrator(&pool, &fixed, "mig_fail").await.unwrap();
    assert_eq!(recorded(&pool, "mig_fail").await, vec![1, 2]);
    assert!(exists(&pool, "mig_fail.second").await);
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM mig_fail.counter")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 1, "file 1 was not applied twice");
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_schema_from_before_the_history_is_refused_untouched(pool: PgPool) {
    sqlx::query("CREATE SCHEMA mig_legacy")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("CREATE TABLE mig_legacy.users (id int)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO mig_legacy.users VALUES (7)")
        .execute(&pool)
        .await
        .unwrap();
    let files = migrator(&[(1, "CREATE TABLE mig_legacy.users (id int);")]);
    let error = run_migrator(&pool, &files, "mig_legacy")
        .await
        .expect_err("tables without a history");
    assert!(
        error.to_string().contains("no migration history"),
        "{error}"
    );
    let kept: i32 = sqlx::query_scalar("SELECT id FROM mig_legacy.users")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(kept, 7, "nothing was changed");
    assert!(!exists(&pool, "mig_legacy._sqlx_migrations").await);
}

#[sqlx::test(migrator = "crate::common::GUARD")]
async fn migrations_are_safe_to_apply_again_over_existing_data(pool: PgPool) {
    let world = &World::new(pool).await;
    let workspace_id = world.workspace_id;
    let (project_id, _) = new_project(world, workspace_id, "Migrate twice").await;
    let (s, v) = post_as(
        world,
        &world.owner_cookie,
        &format!("/v1/projects/{project_id}/views"),
        "p6mg-view",
        json!({"name":"Before","layout":"TABLE"}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM penfii.project_views")
        .fetch_one(&world.pool)
        .await
        .unwrap();
    // Starting the server again runs every migration again; nothing may fail or change.
    penfii_core::apply_migrations(&world.pool)
        .await
        .expect("second run");
    penfii_core::apply_migrations(&world.pool)
        .await
        .expect("third run");
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM penfii.project_views")
        .fetch_one(&world.pool)
        .await
        .unwrap();
    assert_eq!(before, after);
    let layout: String = sqlx::query_scalar(
        "SELECT layout FROM penfii.project_views WHERE name='Before' AND project_id=$1",
    )
    .bind(Uuid::parse_str(&project_id).unwrap())
    .fetch_one(&world.pool)
    .await
    .unwrap();
    assert_eq!(layout, "TABLE");
    // A legacy row (schema v1, no display, Board only) still reads back with defaults.
    sqlx::query("INSERT INTO penfii.project_views(id,workspace_id,project_id,owner_id,scope,name,layout,definition_schema_version,definition) VALUES($1,$2,$3,$4,'PERSONAL','Legacy','BOARD',1,'{\"filter\":{},\"group\":\"STATUS\",\"sort\":{\"field\":\"RANK\",\"dir\":\"ASC\"}}'::jsonb)")
        .bind(Uuid::new_v4()).bind(workspace_id).bind(Uuid::parse_str(&project_id).unwrap()).bind(world.owner_id).execute(&world.pool).await.unwrap();
    let (s, views) = get_as(
        world,
        &world.owner_cookie,
        &format!("/v1/projects/{project_id}/views"),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{views}");
    let legacy = views
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == "Legacy")
        .unwrap();
    assert_eq!(legacy["targetKind"], "PROJECT");
    assert_eq!(legacy["layout"], "BOARD");
    // The shape constraints the migration added are real.
    let bad = sqlx::query("INSERT INTO penfii.project_views(id,workspace_id,project_id,owner_id,scope,name,layout,definition) VALUES($1,$2,NULL,$3,'PERSONAL','NoTarget','LIST','{}'::jsonb)")
        .bind(Uuid::new_v4()).bind(workspace_id).bind(world.owner_id).execute(&world.pool).await;
    assert!(
        bad.is_err(),
        "a View with no Project, Team or Workspace target is refused by the database"
    );
    let gantt = sqlx::query("INSERT INTO penfii.project_views(id,workspace_id,project_id,owner_id,scope,name,layout,definition) VALUES($1,$2,$3,$4,'PERSONAL','Gantt','GANTT','{}'::jsonb)")
        .bind(Uuid::new_v4()).bind(workspace_id).bind(Uuid::parse_str(&project_id).unwrap()).bind(world.owner_id).execute(&world.pool).await;
    assert!(
        gantt.is_err(),
        "an unknown layout is refused by the database"
    );
    // Historic assignments without an outcome stay readable.
    let (s, plan, _) = call(
        &world.app,
        "GET",
        &format!("/v1/projects/{project_id}/planning"),
        Some(&world.owner_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{plan}");
}

/// `CREATE INDEX CONCURRENTLY` is refused inside a transaction and accepted in a no-transaction
/// file. sqlx sends a file as one script, and PostgreSQL runs a multi-statement script as one
/// transaction, so a *failed* script does not leave its earlier statements. The gap that does
/// leave objects behind is a crash after the script and before the history row: the same file,
/// written so it can run again, then applies.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_no_transaction_migration_can_build_an_index_concurrently_and_be_rerun(pool: PgPool) {
    let _turn = crate::common::MIGRATING.acquire().await.unwrap();
    let refused = run_migrator(
        &pool,
        &migrator_tx(
            &[(
                1,
                "CREATE TABLE mig_txidx.t (n int); CREATE INDEX CONCURRENTLY t_n ON mig_txidx.t (n);",
            )],
            false,
        ),
        "mig_txidx",
    )
    .await
    .expect_err("CONCURRENTLY inside the transactional file");
    assert!(
        refused.to_string().to_lowercase().contains("concurrently"),
        "{refused}"
    );
    assert!(!exists(&pool, "mig_txidx.t").await);

    let table = "CREATE TABLE mig_notx.t (n int);";
    run_migrator(&pool, &migrator(&[(1, table)]), "mig_notx")
        .await
        .unwrap();
    let index = "CREATE INDEX CONCURRENTLY IF NOT EXISTS t_n ON mig_notx.t (n);";
    run_migrator(
        &pool,
        &migrator_tx(&[(1, table), (2, index)], true),
        "mig_notx",
    )
    .await
    .unwrap();
    let valid: bool = sqlx::query_scalar(
        "SELECT indisvalid FROM pg_index i JOIN pg_class c ON c.oid = i.indexrelid
         WHERE c.relname = 't_n'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(valid, "the index was built outside a transaction");
    // The script committed and the history row did not (a crash in between).
    sqlx::query("DELETE FROM mig_notx._sqlx_migrations WHERE version = 2")
        .execute(&pool)
        .await
        .unwrap();
    run_migrator(
        &pool,
        &migrator_tx(&[(1, table), (2, index)], true),
        "mig_notx",
    )
    .await
    .unwrap();
    assert_eq!(recorded(&pool, "mig_notx").await, vec![1, 2]);
}
