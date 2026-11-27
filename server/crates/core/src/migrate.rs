use std::time::Duration;

use sqlx::PgPool;
use sqlx::migrate::{Migrate, Migrator};
use sqlx::postgres::PgPoolOptions;

/// Connections in the pool when `PENFII_DB_MAX_CONNECTIONS` is not set.
pub const DEFAULT_MAX_CONNECTIONS: u32 = 10;
/// The most `PENFII_DB_MAX_CONNECTIONS` accepts. PostgreSQL itself defaults to 100 connections for
/// everything that connects to it, so a larger pool is a mistake rather than a tuning.
const MAX_CONNECTIONS_LIMIT: u32 = 500;

/// Reads `PENFII_DB_MAX_CONNECTIONS` (a positive integer). Unset, empty and blank mean the default (Compose
/// passes an empty value when the variable is not in `.env`); anything else
/// that is not a number in range is an error, so a typo is found at start-up.
pub fn max_connections_from_env() -> Result<u32, String> {
    max_connections(std::env::var("PENFII_DB_MAX_CONNECTIONS").ok().as_deref())
}

fn max_connections(raw: Option<&str>) -> Result<u32, String> {
    let Some(raw) = raw.filter(|value| !value.trim().is_empty()) else {
        return Ok(DEFAULT_MAX_CONNECTIONS);
    };
    raw.trim()
        .parse::<u32>()
        .ok()
        .filter(|value| (1..=MAX_CONNECTIONS_LIMIT).contains(value))
        .ok_or_else(|| {
            format!(
                "PENFII_DB_MAX_CONNECTIONS must be a whole number from 1 to {MAX_CONNECTIONS_LIMIT}, got {raw:?}"
            )
        })
}

/// One pool for the whole process: the request handlers and every background task (event
/// consumers, reminders, retention, search maintenance) draw from it, each holding a connection
/// only for the length of one query or transaction. The co-editing owner keeps one more connection
/// of its own outside the pool, so PostgreSQL sees up to `max + 1` per core-api process.
pub async fn connect(database_url: &str) -> Result<PgPool, sqlx::Error> {
    let max =
        max_connections_from_env().map_err(|message| sqlx::Error::Configuration(message.into()))?;
    PgPoolOptions::new()
        .max_connections(max)
        .acquire_timeout(Duration::from_secs(10))
        .connect(database_url)
        .await
}

/// The connection string migrations run with (`PENFII_MIGRATION_DATABASE_URL`): the credential of the
/// owner of the schema, which core-api never holds. `None` when it is not set.
pub fn migration_url_from_env() -> Option<String> {
    std::env::var("PENFII_MIGRATION_DATABASE_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Checks, with the rights of the runtime role (it only reads the history), that the database has
/// every migration of this build, applied and unchanged. core-api and the `admin-cli` commands do
/// not migrate; they call this at start and stop with a message that says what to run
/// (`admin-cli migrate`, with the migration credential) when the schema is behind.
pub async fn require_current_schema(pool: &PgPool) -> Result<(), String> {
    let history: Result<Vec<(i64, Vec<u8>, bool)>, sqlx::Error> =
        sqlx::query_as("SELECT version, checksum, success FROM penfii._sqlx_migrations")
            .fetch_all(pool)
            .await;
    let applied = match history {
        Ok(rows) => rows,
        Err(error) => {
            let missing = error
                .as_database_error()
                .and_then(|e| e.code())
                .is_some_and(|code| code == "42P01" || code == "3F000");
            return Err(if missing {
                "the database has not been migrated (there is no migration history). Run \
                 `admin-cli migrate` with PENFII_MIGRATION_DATABASE_URL set, then start again"
                    .to_string()
            } else {
                format!("cannot read the migration history: {error}")
            });
        }
    };
    let split: Vec<(i64, String, bool)> = applied
        .iter()
        .map(|(version, checksum, success)| (*version, hex::encode(checksum), *success))
        .collect();
    for set in SPLITS {
        if holds_split(set, &split) {
            return Err(format!(
                "the migration history holds the pre-release split migrations {}, which this build \
                 has as one {}. Run `admin-cli migrate` with PENFII_MIGRATION_DATABASE_URL set (it \
                 rewrites that history when it is complete and unchanged), then start again",
                set.range, set.consolidated
            ));
        }
    }
    for migration in MIGRATOR
        .iter()
        .filter(|m| !m.migration_type.is_down_migration())
    {
        match applied
            .iter()
            .find(|(version, _, _)| *version == migration.version)
        {
            None => {
                return Err(format!(
                    "the database is behind this build: migration {} ({}) has not been applied. Run \
                     `admin-cli migrate` with PENFII_MIGRATION_DATABASE_URL set, then start again",
                    migration.version, migration.description
                ));
            }
            Some((_, _, false)) => {
                return Err(format!(
                    "migration {} ({}) is recorded as failed; fix the database, then run \
                     `admin-cli migrate`",
                    migration.version, migration.description
                ));
            }
            Some((_, checksum, true)) if checksum.as_slice() != migration.checksum.as_ref() => {
                return Err(format!(
                    "migration {} ({}) was changed after it was applied; this build and the \
                     database do not belong together",
                    migration.version, migration.description
                ));
            }
            Some(_) => {}
        }
    }
    Ok(())
}

/// After a migration run: the runtime role may read the migration history but never write it, and it
/// may append to and read the audit log but never change or delete an entry, even though the default
/// privileges of the schema give it the DML rights on every new table (both tables are made by a
/// migration run). Takes those rights away from every role but the owner. Safe to run again.
pub async fn protect_history(pool: &PgPool) -> Result<(), sqlx::Error> {
    for (table, revoked) in PROTECTED_TABLES {
        sqlx::query(&format!(
            "DO $$
             DECLARE grantee_name text;
             BEGIN
                 IF to_regclass('penfii.{table}') IS NULL THEN RETURN; END IF;
                 FOR grantee_name IN
                     SELECT DISTINCT pg_get_userbyid(a.grantee)
                     FROM pg_class c, LATERAL aclexplode(c.relacl) a
                     WHERE c.oid = 'penfii.{table}'::regclass
                       AND a.grantee <> 0 AND a.grantee <> c.relowner
                       AND a.privilege_type IN ({privileges})
                 LOOP
                     EXECUTE format(
                         'REVOKE {revoked} ON penfii.{table} FROM %I',
                         grantee_name);
                 END LOOP;
             END $$",
            privileges = quoted_list(revoked),
        ))
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// The tables whose rows the runtime role must not change, with the rights it loses on them.
/// `_sqlx_migrations` is read only; `audit_entries` is append only (INSERT and SELECT stay).
pub(crate) const PROTECTED_TABLES: [(&str, &str); 2] = [
    ("_sqlx_migrations", "INSERT, UPDATE, DELETE, TRUNCATE"),
    ("audit_entries", "UPDATE, DELETE, TRUNCATE"),
];

fn quoted_list(privileges: &str) -> String {
    privileges
        .split(", ")
        .map(|name| format!("'{name}'"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The schema every Penfii table lives in. The migration history table lives in it too, so
/// dropping the schema drops the history with the tables.
pub(crate) const SCHEMA: &str = "penfii";

/// The numbered files in `database/migrations/`, compiled into the binary.
static MIGRATOR: Migrator = sqlx::migrate!("../../../database/migrations");

/// Applies every migration that has not been applied yet.
///
/// sqlx takes a PostgreSQL advisory lock for the run (a second instance waits and then finds
/// nothing to do), records each version with the checksum of its file in `_sqlx_migrations`,
/// refuses to continue when an applied file was edited, runs each file in a transaction unless the
/// file starts with `-- no-transaction` (needed for `CREATE INDEX CONCURRENTLY`; sqlx sends the
/// file as one script, so a failed script still rolls back, while a crash after the script and
/// before its history row does not), and never runs a version twice.
///
/// Before that, a history recorded by a development build with files that this build has merged into
/// one (the split 0003-0007, the split 0004-0005) is turned into the one file's row
/// ([`consolidate_split`]); the result says which were.
pub async fn apply_migrations(pool: &PgPool) -> Result<MigrationRun, sqlx::Error> {
    run(pool, &MIGRATOR, SCHEMA, true).await
}

/// What [`apply_migrations`] did besides applying pending files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MigrationRun {
    /// The history held the five split files 0003-0007, applied and unchanged, and now holds the
    /// consolidated `0003` instead (nothing was applied again).
    pub consolidated_split_0003: bool,
    /// The history held the two split files 0004 (long sessions) and 0005 (doc full width), applied
    /// and unchanged, and now holds the consolidated `0004` instead (nothing was applied again).
    pub consolidated_split_0004: bool,
}

/// [`apply_migrations`] for any migrator and schema; the tests use it with their own. It never
/// rewrites a history.
pub async fn run_migrator(
    pool: &PgPool,
    migrator: &Migrator,
    schema: &str,
) -> Result<(), sqlx::Error> {
    run(pool, migrator, schema, false).await.map(|_| ())
}

async fn run(
    pool: &PgPool,
    migrator: &Migrator,
    schema: &str,
    consolidate: bool,
) -> Result<MigrationRun, sqlx::Error> {
    let mut conn = pool.acquire().await?;
    // Two instances may start together; the loser of the race finds the schema already there.
    if let Err(error) = sqlx::query(&format!("CREATE SCHEMA IF NOT EXISTS {schema}"))
        .execute(&mut *conn)
        .await
        && !matches!(
            error.as_database_error().and_then(|e| e.code()).as_deref(),
            Some("42P06" | "23505")
        )
    {
        return Err(error);
    }
    // `_sqlx_migrations` is created in the first schema of the search path.
    sqlx::query(&format!("SET search_path TO {schema}, public"))
        .execute(&mut *conn)
        .await?;
    let outcome = guarded_run(&mut conn, migrator, schema, consolidate).await;
    // sqlx takes a session-level advisory lock and releases it only when the run succeeds; after an
    // error the lock would stay with this pooled connection and block every later run. Closing the
    // connection releases it (and also drops the changed search path).
    if outcome.is_err()
        || sqlx::query("RESET search_path")
            .execute(&mut *conn)
            .await
            .is_err()
    {
        drop(conn.detach());
    }
    outcome
}

async fn guarded_run(
    conn: &mut sqlx::PgConnection,
    migrator: &Migrator,
    schema: &str,
    consolidate: bool,
) -> Result<MigrationRun, sqlx::Error> {
    // A database made before the migration history was reset (the development migrations 0001-0030)
    // has the tables but no history. Running the first file over it would fail halfway on "already
    // exists"; say what to do instead.
    let (tables, history): (i64, bool) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM pg_tables WHERE schemaname = $1 AND tablename <> '_sqlx_migrations'),
                to_regclass($1 || '._sqlx_migrations') IS NOT NULL",
    )
    .bind(schema)
    .fetch_one(&mut *conn)
    .await?;
    if tables > 0 && !history {
        return Err(sqlx::Error::Protocol(format!(
            "schema {schema:?} has tables but no migration history. It was made by a development build \
             from before the migration history was reset to 0001_initial.sql; recreate the database \
             (see docs/development.md, \"Resetting a development database\")."
        )));
    }
    let (mut consolidated_split_0003, mut consolidated_split_0004) = (false, false);
    if consolidate && history {
        // Under the migrator's own advisory lock (PostgreSQL advisory locks nest within a session, so
        // `run_direct` below takes it again and releases it once; the second release is ours). A second
        // instance waits here and then finds the history already consolidated. After an error the
        // caller closes the connection, which releases both.
        conn.lock()
            .await
            .map_err(|error| sqlx::Error::Migrate(Box::new(error)))?;
        consolidated_split_0003 = consolidate_split(conn, migrator, schema, &SPLIT_0003).await?;
        consolidated_split_0004 = consolidate_split(conn, migrator, schema, &SPLIT_0004).await?;
    }
    // `run_direct` is what `run` calls once it has a connection; `run` itself cannot be spawned onto
    // another task because of how `Acquire` is declared.
    migrator
        .run_direct(&mut *conn)
        .await
        .map_err(|error| sqlx::Error::Migrate(Box::new(error)))?;
    if consolidate && history {
        conn.unlock()
            .await
            .map_err(|error| sqlx::Error::Migrate(Box::new(error)))?;
    }
    Ok(MigrationRun {
        consolidated_split_0003,
        consolidated_split_0004,
    })
}

/// A set of development migration files that a later build merged into one file, and what is needed to
/// recognise the history they left and to rewrite it.
struct Split {
    /// The versions of the files, as the text of a message ("0003-0007").
    range: &'static str,
    /// The file that replaced them, as the text of a message ("0003").
    consolidated: &'static str,
    /// The version of the merged file. It is also the version of the first split file, whose row is
    /// kept (and rewritten); the rows of the others go.
    version: i64,
    /// Version, description and the SHA-384 checksum sqlx recorded for each split file.
    files: &'static [(i64, &'static str, &'static str)],
    /// Every version a history of such a build holds, in order: this build's own files before the
    /// split ones, then the split ones.
    versions: &'static [i64],
    /// `SELECT` of one boolean, with the schema as `$1`: the objects the split files made are there.
    objects: &'static str,
}

/// The files a development build had between v0.3.1 and the next release (never released): the five
/// that made up what is now `0003_after_v0_3_1.sql`. Together they made exactly the schema of the
/// consolidated `0003`, and their data clean-ups (0004, 0007) are the one in it. The entry can go once
/// no such database is left.
const SPLIT_0003: Split = Split {
    range: "0003-0007",
    consolidated: "0003",
    version: 3,
    files: &[
        (
            3,
            "email change",
            "d1bf16453967b4e72224570456c91f27bd8dfe98de9284131c025631bae0a6b7faed06a48b57ff3a5ea00c40394f8aa0",
        ),
        (
            4,
            "removed member access",
            "5d5e9fc0ed4ece384378d3b4a8026216ecaf6a6d53582fcc4618cd9bc976b7c44f0592cba1b6c4e15760540ca6b217c7",
        ),
        (
            5,
            "auth attempt browser",
            "80950a04f727d822824cdffd468fd3c607a2cf6ea88000af8f082a16ce89b9e48caa5cdf192aaf475ae1dd29dd56c2fe",
        ),
        (
            6,
            "mfa totp",
            "1f2948871460745f4853df52afb23f68610d8a21be58df18dabf70cbe19a18ad64ad745e2b63c5e3727c9bb4a2bf8960",
        ),
        (
            7,
            "removed member access followup",
            "84a73e836c0525331351d8b9607a91986f03543c6a6d493c8f36aadb39b228848f153aca829de97310bcd777497489da",
        ),
    ],
    versions: &[1, 2, 3, 4, 5, 6, 7],
    // The tables and the column the split files made.
    objects: "SELECT to_regclass($1 || '.email_change_requests') IS NOT NULL
        AND to_regclass($1 || '.mfa_instance_key') IS NOT NULL
        AND to_regclass($1 || '.mfa_totp') IS NOT NULL
        AND to_regclass($1 || '.mfa_recovery_codes') IS NOT NULL
        AND EXISTS (SELECT 1 FROM information_schema.columns
                    WHERE table_schema = $1 AND table_name = 'auth_attempts' AND column_name = 'browser_hash')",
};

/// The two files a development build had after v0.4.1 (never released): `0004_long_sessions.sql` and
/// `0005_doc_full_width.sql`, now `0004_after_v0_4_1.sql`. Same schema and no data change; the entry
/// can go once no such database is left.
const SPLIT_0004: Split = Split {
    range: "0004-0005",
    consolidated: "0004",
    version: 4,
    files: &[
        (
            4,
            "long sessions",
            "4e00073cdf579c517d6f01b6b881d82bef2c70f0b5aad44efa989dcc08d3b5cfcf73ce1e0235c200214daff4ccc50581",
        ),
        (
            5,
            "doc full width",
            "bff8c7397ce65c3b067d5867a3cadc77de595eff2572a022881d2c737502adcafac91ae57d4542a9b16a634a5bf768b1",
        ),
    ],
    versions: &[1, 2, 3, 4, 5],
    // The columns the two files added.
    objects: "SELECT (SELECT count(*) FROM information_schema.columns
                    WHERE table_schema = $1
                      AND (table_name, column_name) IN (('sessions', 'absolute_expires_at'),
                                                        ('sessions', 'idle_seconds'),
                                                        ('auth_attempts', 'keep_signed_in'),
                                                        ('doc_pages', 'full_width'))) = 4",
};

const SPLITS: &[&Split] = &[&SPLIT_0003, &SPLIT_0004];

/// Whether a recorded history is that split one, judged by checksums (never by version numbers alone:
/// a later release may have its own 0004).
fn holds_split(set: &Split, rows: &[(i64, String, bool)]) -> bool {
    rows.iter().any(|(version, checksum, _)| {
        set.files
            .iter()
            .any(|(v, _, c)| v == version && c == checksum)
    })
}

/// Replaces the history rows of the split files with the one row of the consolidated file, when the
/// history is exactly what such a build left: the versions of `set.versions`, all applied, this build's
/// own files with this build's checksums, the split ones with the checksums of the split files, and the
/// objects they made present. Nothing runs again and no data changes; the rows are rewritten in one
/// transaction. A history that holds only some of the split files, or anything else alongside them, is
/// refused with a message and left as it is. Returns whether it rewrote the history.
async fn consolidate_split(
    conn: &mut sqlx::PgConnection,
    migrator: &Migrator,
    schema: &str,
    set: &Split,
) -> Result<bool, sqlx::Error> {
    let rows: Vec<(i64, String, bool)> = sqlx::query_as(&format!(
        "SELECT version, encode(checksum, 'hex'), success FROM {schema}._sqlx_migrations ORDER BY version"
    ))
    .fetch_all(&mut *conn)
    .await?;
    if !holds_split(set, &rows) {
        return Ok(false);
    }
    let (range, consolidated) = (set.range, set.consolidated);
    let refuse = |why: String| {
        sqlx::Error::Protocol(format!(
            "the migration history holds the pre-release split migrations {range}, but {why}. It was \
             left unchanged. Restore the database from the backup taken before that build, or recreate \
             it (docs/development.md, \"Resetting a development database\")."
        ))
    };
    let own = |version: i64| {
        migrator
            .iter()
            .find(|m| m.version == version && !m.migration_type.is_down_migration())
    };
    let target = own(set.version)
        .ok_or_else(|| refuse(format!("this build has no migration {consolidated}")))?;
    let target_checksum = hex::encode(&target.checksum);
    if set.files.iter().any(|(_, _, c)| *c == target_checksum) {
        return Err(refuse("this build still has the split files".into()));
    }
    let versions: Vec<i64> = rows.iter().map(|(v, _, _)| *v).collect();
    if versions != set.versions {
        return Err(refuse(format!(
            "the recorded versions are {versions:?}, not {:?}",
            set.versions
        )));
    }
    for (version, checksum, success) in &rows {
        if !success {
            return Err(refuse(format!("migration {version} is recorded as failed")));
        }
        let expected = match set.files.iter().find(|(v, _, _)| v == version) {
            Some((_, _, split)) => (*split).to_string(),
            None => own(*version)
                .map(|m| hex::encode(&m.checksum))
                .ok_or_else(|| refuse(format!("this build has no migration {version}")))?,
        };
        if *checksum != expected {
            return Err(refuse(format!(
                "migration {version} does not have the checksum of that build"
            )));
        }
    }
    let present: bool = sqlx::query_scalar(set.objects)
        .bind(schema)
        .fetch_one(&mut *conn)
        .await?;
    if !present {
        return Err(refuse(
            "the tables or the columns those files made are missing".into(),
        ));
    }

    let mut tx = sqlx::Connection::begin(&mut *conn).await?;
    sqlx::query(&format!(
        "LOCK TABLE {schema}._sqlx_migrations IN ACCESS EXCLUSIVE MODE"
    ))
    .execute(&mut *tx)
    .await?;
    let others = set.files.len() as u64 - 1;
    let mut removed = 0;
    for (version, _, checksum) in set.files.iter().filter(|(v, _, _)| *v != set.version) {
        removed += sqlx::query(&format!(
            "DELETE FROM {schema}._sqlx_migrations
             WHERE version = $1 AND checksum = decode($2, 'hex') AND success"
        ))
        .bind(version)
        .bind(checksum)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    }
    let updated = sqlx::query(&format!(
        "UPDATE {schema}._sqlx_migrations
            SET description = $1, checksum = $2
          WHERE version = $4 AND checksum = decode($3, 'hex') AND success"
    ))
    .bind(target.description.as_ref())
    .bind(target.checksum.as_ref())
    .bind(set.files[0].2)
    .bind(set.version)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if removed != others || updated != 1 {
        tx.rollback().await?;
        return Err(refuse(format!(
            "the rewrite matched {removed} of {others} rows to remove and {updated} of 1 to replace"
        )));
    }
    tx.commit().await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pool_defaults_to_ten_and_takes_a_number_in_range() {
        assert_eq!(max_connections(None), Ok(10));
        // Empty and blank are "not set", like every other setting.
        assert_eq!(max_connections(Some("")), Ok(10));
        assert_eq!(max_connections(Some("  ")), Ok(10));
        assert_eq!(max_connections(Some("25")), Ok(25));
        assert_eq!(max_connections(Some(" 1 ")), Ok(1));
        assert_eq!(max_connections(Some("500")), Ok(500));
    }

    #[test]
    fn a_pool_size_that_is_not_a_whole_number_in_range_is_an_error() {
        for bad in ["0", "-3", "501", "ten", "2.5", "1e2"] {
            let error = max_connections(Some(bad)).unwrap_err();
            assert!(
                error.contains("PENFII_DB_MAX_CONNECTIONS"),
                "{bad}: {error}"
            );
        }
    }
}
