use std::time::Duration;

use sqlx::PgPool;
use sqlx::migrate::Migrator;
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

/// After a migration run: the runtime role may read the migration history but never write it, even
/// though the default privileges of the schema give it the DML rights on every new table (the history
/// table is made by the first migration run). Takes those rights away from every role but the
/// owner. Safe to run again.
pub async fn protect_history(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "DO $$
         DECLARE grantee_name text;
         BEGIN
             FOR grantee_name IN
                 SELECT DISTINCT pg_get_userbyid(a.grantee)
                 FROM pg_class c, LATERAL aclexplode(c.relacl) a
                 WHERE c.oid = 'penfii._sqlx_migrations'::regclass
                   AND a.grantee <> 0 AND a.grantee <> c.relowner
                   AND a.privilege_type IN ('INSERT', 'UPDATE', 'DELETE', 'TRUNCATE')
             LOOP
                 EXECUTE format(
                     'REVOKE INSERT, UPDATE, DELETE, TRUNCATE ON penfii._sqlx_migrations FROM %I',
                     grantee_name);
             END LOOP;
         END $$",
    )
    .execute(pool)
    .await?;
    Ok(())
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
pub async fn apply_migrations(pool: &PgPool) -> Result<(), sqlx::Error> {
    run_migrator(pool, &MIGRATOR, SCHEMA).await
}

/// [`apply_migrations`] for any migrator and schema; the tests use it with their own.
pub async fn run_migrator(
    pool: &PgPool,
    migrator: &Migrator,
    schema: &str,
) -> Result<(), sqlx::Error> {
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
    let outcome = guarded_run(&mut conn, migrator, schema).await;
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
) -> Result<(), sqlx::Error> {
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
    // `run_direct` is what `run` calls once it has a connection; `run` itself cannot be spawned onto
    // another task because of how `Acquire` is declared.
    migrator
        .run_direct(&mut *conn)
        .await
        .map_err(|error| sqlx::Error::Migrate(Box::new(error)))
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
