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
    assert_eq!(recorded(&pool, "penfii").await, vec![1, 2, 3]);
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
    assert_eq!(recorded(&pool, "penfii").await, vec![1, 2, 3]);
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
    assert_eq!(recorded(&pool, "penfii").await, vec![1, 2, 3]);
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

/// v0.1.0 shipped `0001_initial.sql` alone. A database it made (history `[1]`, checksum of that file)
/// takes `0002` in place: the history is accepted, the rows stay, and the data clean-ups of `0002` run
/// (project channel rows of people no longer in the Project, piled-up open invitations).
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn a_v0_1_0_database_upgrades_in_place_and_keeps_its_rows(pool: PgPool) {
    let _turn = crate::common::MIGRATING.acquire().await.unwrap();
    // The history v0.1.0 recorded: version 1 with the checksum of the released file.
    let v0_1_0 = Migrator {
        migrations: Cow::Owned(vec![Migration::new(
            1,
            Cow::Borrowed("initial"),
            MigrationType::Simple,
            Cow::Borrowed(include_str!(
                "../../../database/migrations/0001_initial.sql"
            )),
            false,
        )]),
        ..Migrator::DEFAULT
    };
    run_migrator(&pool, &v0_1_0, "penfii").await.unwrap();
    assert_eq!(recorded(&pool, "penfii").await, vec![1]);

    let (ws, project) = (Uuid::new_v4(), Uuid::new_v4());
    let (owner, kept, gone) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    let (private, public) = (Uuid::new_v4(), Uuid::new_v4());
    let (older, newer, accepted) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    sqlx::raw_sql(&format!(
        "INSERT INTO penfii.users (id, email, email_key, display_name) VALUES
             ('{owner}', 'owner@example.com', 'owner@example.com', 'Owner'),
             ('{kept}', 'kept@example.com', 'kept@example.com', 'Kept'),
             ('{gone}', 'gone@example.com', 'gone@example.com', 'Gone');
         INSERT INTO penfii.workspaces (id, name, time_zone) VALUES ('{ws}', 'Upgrade', 'Asia/Seoul');
         INSERT INTO penfii.workspace_members (workspace_id, user_id, role) VALUES
             ('{ws}', '{owner}', 'OWNER'), ('{ws}', '{kept}', 'MEMBER'), ('{ws}', '{gone}', 'MEMBER');
         INSERT INTO penfii.projects (id, workspace_id, name, ticket_prefix) VALUES ('{project}', '{ws}', 'P', 'UPG');
         INSERT INTO penfii.project_user_grants (project_id, user_id, role) VALUES
             ('{project}', '{owner}', 'ADMIN'), ('{project}', '{kept}', 'EDITOR');
         INSERT INTO penfii.conversations (id, workspace_id, kind) VALUES
             ('{private}', '{ws}', 'CHANNEL'), ('{public}', '{ws}', 'CHANNEL');
         INSERT INTO penfii.channels (id, project_id, name, visibility) VALUES
             ('{private}', '{project}', 'private', 'PRIVATE'), ('{public}', '{project}', 'public', 'PUBLIC');
         -- `gone` lost the Project's grant in v0.1.0, which left the channel rows behind.
         INSERT INTO penfii.channel_members (channel_id, user_id, role) VALUES
             ('{private}', '{owner}', 'MANAGER'), ('{private}', '{kept}', 'MEMBER'),
             ('{private}', '{gone}', 'MANAGER'), ('{public}', '{gone}', 'MEMBER');
         -- v0.1.0 let open invitations for one address pile up.
         INSERT INTO penfii.workspace_invitations
             (id, workspace_id, email_key, role, token_hash, expires_at, created_by, created_at) VALUES
             ('{older}', '{ws}', 'new@example.com', 'MEMBER', '\\x01', now() + interval '7 days', '{owner}', now() - interval '2 days'),
             ('{newer}', '{ws}', 'new@example.com', 'MEMBER', '\\x02', now() + interval '7 days', '{owner}', now() - interval '1 day');
         INSERT INTO penfii.workspace_invitations
             (id, workspace_id, email_key, role, token_hash, expires_at, created_by, accepted_user_id, accepted_at) VALUES
             ('{accepted}', '{ws}', 'kept@example.com', 'MEMBER', '\\x03', now() + interval '7 days', '{owner}', '{kept}', now());"
    ))
    .execute(&pool)
    .await
    .unwrap();

    // What admin-cli migrate does on the upgrade: 0002 and the later files are applied, and the build accepts the result.
    apply_migrations(&pool).await.expect("0002 over v0.1.0");
    assert_eq!(recorded(&pool, "penfii").await, vec![1, 2, 3]);
    penfii_core::require_current_schema(&pool)
        .await
        .expect("the history matches this build");

    let users: Vec<(Uuid, String)> =
        sqlx::query_as("SELECT id, kind FROM penfii.users ORDER BY email")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(users.len(), 3);
    assert!(users.iter().all(|(_, kind)| kind == "HUMAN"), "{users:?}");
    let members: Vec<(Uuid, Uuid, String)> = sqlx::query_as(
        "SELECT channel_id, user_id, role FROM penfii.channel_members ORDER BY role, user_id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let mut expected = vec![
        (private, owner, "MANAGER".to_string()),
        (private, kept, "MEMBER".to_string()),
    ];
    expected.sort_by(|a, b| (&a.2, a.1).cmp(&(&b.2, b.1)));
    assert_eq!(members, expected, "only the rows of `gone` went");
    let invitations: Vec<(Uuid, bool, Option<String>)> = sqlx::query_as(
        "SELECT id, revoked_at IS NOT NULL, email_status FROM penfii.workspace_invitations ORDER BY created_at",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(invitations.len(), 3);
    for (id, revoked, email_status) in &invitations {
        assert_eq!(*revoked, *id == older, "{id}");
        assert_eq!(*email_status, None);
    }
    // The new constraint holds from now on: a second open invitation for the address is refused.
    let duplicate = sqlx::raw_sql(&format!(
        "INSERT INTO penfii.workspace_invitations (id, workspace_id, email_key, role, token_hash, expires_at, created_by)
         VALUES ('{}', '{ws}', 'new@example.com', 'MEMBER', '\\x04', now() + interval '1 day', '{owner}')",
        Uuid::new_v4()
    ))
    .execute(&pool)
    .await;
    assert!(duplicate.is_err());
    // Applying again changes nothing.
    apply_migrations(&pool).await.expect("second run");
    assert_eq!(recorded(&pool, "penfii").await, vec![1, 2, 3]);
}

/// The migrations a development build between v0.3.1 and the consolidated 0003 recorded: 0001 and 0002
/// of this build, then the five split files (fixtures, byte for byte as that build had them).
fn split_0003_build() -> Migrator {
    let file = |version: i64, description: &'static str, sql: &'static str| {
        Migration::new(
            version,
            Cow::Borrowed(description),
            MigrationType::Simple,
            Cow::Borrowed(sql),
            false,
        )
    };
    Migrator {
        migrations: Cow::Owned(vec![
            file(
                1,
                "initial",
                include_str!("../../../database/migrations/0001_initial.sql"),
            ),
            file(
                2,
                "after v0 1 0",
                include_str!("../../../database/migrations/0002_after_v0_1_0.sql"),
            ),
            file(
                3,
                "email change",
                include_str!("fixtures/split_0003/0003_email_change.sql"),
            ),
            file(
                4,
                "removed member access",
                include_str!("fixtures/split_0003/0004_removed_member_access.sql"),
            ),
            file(
                5,
                "auth attempt browser",
                include_str!("fixtures/split_0003/0005_auth_attempt_browser.sql"),
            ),
            file(
                6,
                "mfa totp",
                include_str!("fixtures/split_0003/0006_mfa_totp.sql"),
            ),
            file(
                7,
                "removed member access followup",
                include_str!("fixtures/split_0003/0007_removed_member_access_followup.sql"),
            ),
        ]),
        ..Migrator::DEFAULT
    }
}

async fn history(pool: &PgPool) -> Vec<(i64, String, Vec<u8>)> {
    sqlx::query_as(
        "SELECT version, description, checksum FROM penfii._sqlx_migrations ORDER BY version",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

/// The checksum this build has for its 0003.
fn consolidated_checksum() -> Vec<u8> {
    use sha2::Digest;
    sha2::Sha384::digest(
        include_str!("../../../database/migrations/0003_after_v0_3_1.sql").as_bytes(),
    )
    .to_vec()
}

/// A database that a build with the split files migrated: `admin-cli migrate` rewrites its history to
/// the consolidated 0003 without running anything again, the build accepts it, and a second run has
/// nothing to do.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn the_split_0003_history_becomes_the_consolidated_0003(pool: PgPool) {
    let _turn = crate::common::MIGRATING.acquire().await.unwrap();
    run_migrator(&pool, &split_0003_build(), "penfii")
        .await
        .unwrap();
    assert_eq!(recorded(&pool, "penfii").await, vec![1, 2, 3, 4, 5, 6, 7]);
    let error = penfii_core::require_current_schema(&pool)
        .await
        .unwrap_err();
    assert!(error.contains("split migrations 0003-0007"), "{error}");
    // A row the split build wrote stays; the clean-up section is not run again (it would add nothing
    // here, the marker row shows that no table was made again either).
    sqlx::query(
        "INSERT INTO penfii.mfa_instance_key (id, key) VALUES (1, decode(repeat('ab', 32), 'hex'))",
    )
    .execute(&pool)
    .await
    .unwrap();
    let before = history(&pool).await;

    let run = apply_migrations(&pool).await.expect("consolidation");
    assert!(run.consolidated_split_0003);
    let after = history(&pool).await;
    assert_eq!(after.len(), 3);
    assert_eq!(after[..2], before[..2], "0001 and 0002 are untouched");
    assert_eq!(after[2].0, 3);
    assert_eq!(after[2].1, "after v0 3 1");
    assert_eq!(after[2].2, consolidated_checksum());
    penfii_core::require_current_schema(&pool)
        .await
        .expect("the build accepts the consolidated history");
    let keys: i64 = sqlx::query_scalar("SELECT count(*) FROM penfii.mfa_instance_key")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(keys, 1);

    let again = apply_migrations(&pool).await.expect("second run");
    assert!(!again.consolidated_split_0003);
    assert_eq!(history(&pool).await, after);
}

/// Instances that start together on a split history: one rewrites it, the others find it done.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn instances_that_start_together_consolidate_the_split_history_once(pool: PgPool) {
    let _turn = crate::common::MIGRATING.acquire().await.unwrap();
    run_migrator(&pool, &split_0003_build(), "penfii")
        .await
        .unwrap();
    let runs: Vec<_> = (0..4)
        .map(|_| {
            let pool = pool.clone();
            tokio::spawn(async move { apply_migrations(&pool).await })
        })
        .collect();
    let mut rewrote = 0;
    for run in runs {
        if run
            .await
            .unwrap()
            .expect("every instance succeeds")
            .consolidated_split_0003
        {
            rewrote += 1;
        }
    }
    assert_eq!(rewrote, 1);
    assert_eq!(recorded(&pool, "penfii").await, vec![1, 2, 3]);
}

/// A history with only some of the split files, one with a split file changed, and one whose tables
/// are missing are refused and left exactly as they were.
#[sqlx::test(migrator = "crate::common::GUARD")]
async fn an_incomplete_or_changed_split_history_is_refused_untouched(pool: PgPool) {
    let _turn = crate::common::MIGRATING.acquire().await.unwrap();
    let full = split_0003_build();
    let partial = Migrator {
        migrations: Cow::Owned(full.migrations[..5].to_vec()),
        ..Migrator::DEFAULT
    };
    run_migrator(&pool, &partial, "penfii").await.unwrap();
    let before = history(&pool).await;
    let error = apply_migrations(&pool)
        .await
        .expect_err("0006 and 0007 are missing");
    assert!(error.to_string().contains("not 1 to 7"), "{error}");
    assert_eq!(history(&pool).await, before);

    // Completed, then one checksum differs from the split build's.
    run_migrator(&pool, &full, "penfii").await.unwrap();
    sqlx::query("UPDATE penfii._sqlx_migrations SET checksum = '\\x00'::bytea WHERE version = 6")
        .execute(&pool)
        .await
        .unwrap();
    let before = history(&pool).await;
    let error = apply_migrations(&pool).await.expect_err("a changed file");
    assert!(error.to_string().contains("migration 6"), "{error}");
    assert_eq!(history(&pool).await, before);

    // The checksums are right, but a table those files made is gone.
    sqlx::query("UPDATE penfii._sqlx_migrations SET checksum = decode($1, 'hex') WHERE version = 6")
        .bind("1f2948871460745f4853df52afb23f68610d8a21be58df18dabf70cbe19a18ad64ad745e2b63c5e3727c9bb4a2bf8960")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DROP TABLE penfii.mfa_recovery_codes")
        .execute(&pool)
        .await
        .unwrap();
    let before = history(&pool).await;
    let error = apply_migrations(&pool).await.expect_err("missing table");
    assert!(error.to_string().contains("missing"), "{error}");
    assert_eq!(history(&pool).await, before);
}
