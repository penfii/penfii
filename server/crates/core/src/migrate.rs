use std::time::Duration;

use sqlx::PgPool;
use sqlx::migrate::Migrator;
use sqlx::postgres::PgPoolOptions;

/// Connections in the pool when `PENFII_DB_MAX_CONNECTIONS` is not set.
pub const DEFAULT_MAX_CONNECTIONS: u32 = 10;
/// The most `PENFII_DB_MAX_CONNECTIONS` accepts. PostgreSQL itself defaults to 100 connections for
/// everything that connects to it, so a larger pool is a mistake rather than a tuning.
const MAX_CONNECTIONS_LIMIT: u32 = 500;

/// Reads `PENFII_DB_MAX_CONNECTIONS` (a positive integer). Unset means the default; anything else
/// that is not a number in range is an error, so a typo is found at start-up.
pub fn max_connections_from_env() -> Result<u32, String> {
    max_connections(std::env::var("PENFII_DB_MAX_CONNECTIONS").ok().as_deref())
}

fn max_connections(raw: Option<&str>) -> Result<u32, String> {
    let Some(raw) = raw else {
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

/// The schema every Penfii table lives in. The migration history table lives in it too, so
/// dropping the schema drops the history with the tables.
const SCHEMA: &str = "penfii";

/// The numbered files in `database/migrations/`, compiled into the binary.
static MIGRATOR: Migrator = sqlx::migrate!("../../../database/migrations");

/// Applies every migration that has not been applied yet.
///
/// sqlx takes a PostgreSQL advisory lock for the run (a second instance waits and then finds
/// nothing to do), records each version with the checksum of its file in `_sqlx_migrations`,
/// refuses to continue when an applied file was edited, runs each file in a transaction (a failed
/// file leaves nothing behind and is tried again on the next start), and never runs a version
/// twice.
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
        assert_eq!(max_connections(Some("25")), Ok(25));
        assert_eq!(max_connections(Some(" 1 ")), Ok(1));
        assert_eq!(max_connections(Some("500")), Ok(500));
    }

    #[test]
    fn a_pool_size_that_is_not_a_whole_number_in_range_is_an_error() {
        for bad in ["0", "-3", "501", "ten", "", "2.5", "1e2"] {
            let error = max_connections(Some(bad)).unwrap_err();
            assert!(
                error.contains("PENFII_DB_MAX_CONNECTIONS"),
                "{bad}: {error}"
            );
        }
    }
}
