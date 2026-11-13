//! Failed-attempt throttle for password proofs (sign-in and reauthentication). The key is the
//! submitted identifier, whether or not an account exists, so a locked response says nothing about
//! the account.

use chrono::Duration;
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use crate::error::CoreError;
use crate::limits::Bucket;

/// Re-authentication of a signed-in person: 5 failures in 15 minutes, per account.
const REAUTH: Bucket = Bucket {
    max_failures: 5,
    window_seconds: 15 * 60,
};

fn key_hash(key: &str) -> Vec<u8> {
    Sha256::digest(key.as_bytes()).to_vec()
}

/// Fails with `RateLimited` while the key is locked.
pub(crate) async fn check(pool: &PgPool, scope: &str, key: &str) -> Result<(), CoreError> {
    let hash = key_hash(key);
    // Compile-time checked (see server/.sqlx). The lock and its clock are both the database's:
    // the remaining time is worked out there.
    let remaining = sqlx::query_scalar!(
        r#"SELECT EXTRACT(EPOCH FROM (locked_until - now()))::float8
           FROM penfii.auth_throttle WHERE scope = $1 AND key_hash = $2"#,
        scope,
        &hash
    )
    .fetch_optional(pool)
    .await?;
    if let Some(Some(seconds)) = remaining {
        let remaining = seconds as i64;
        if remaining > 0 {
            return Err(CoreError::RateLimited {
                retry_after_seconds: remaining,
            });
        }
    }
    Ok(())
}

pub(crate) async fn record_failure(pool: &PgPool, scope: &str, key: &str) -> Result<(), CoreError> {
    record_failure_limit(pool, scope, key, REAUTH).await
}

/// Counts a failure. The key is locked for one `window_seconds` once the window holds
/// `max_failures`.
pub(crate) async fn record_failure_limit(
    pool: &PgPool,
    scope: &str,
    key: &str,
    bucket: Bucket,
) -> Result<(), CoreError> {
    // Every instant here is the database's clock (`now()`), the one the rows are compared with.
    let hash = key_hash(key);
    let window = bucket.window_seconds as f64;
    sqlx::query!(
        "INSERT INTO penfii.auth_throttle (scope, key_hash, window_start, failures)
         VALUES ($1, $2, now(), 1)
         ON CONFLICT (scope, key_hash) DO UPDATE SET
             failures = CASE WHEN penfii.auth_throttle.window_start < now() - make_interval(secs => $4)
                             THEN 1 ELSE penfii.auth_throttle.failures + 1 END,
             window_start = CASE WHEN penfii.auth_throttle.window_start < now() - make_interval(secs => $4)
                                 THEN now() ELSE penfii.auth_throttle.window_start END,
             locked_until = CASE
                 WHEN penfii.auth_throttle.window_start >= now() - make_interval(secs => $4)
                      AND penfii.auth_throttle.failures + 1 >= $3
                 THEN now() + make_interval(secs => $4) ELSE penfii.auth_throttle.locked_until END",
        scope,
        &hash,
        bucket.max_failures,
        window,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Counts one request against every counter in `counters` at once, or against none: `true` when all of
/// them had room, `false` when any of them is full (nothing is counted then, so a request that is
/// turned away does not push a lock further out).
///
/// Unlike `check` followed by `record_failure_limit`, nothing can come in between: each counter row is
/// locked (`FOR UPDATE`) in the order given, tested and changed in one transaction, so requests that
/// arrive together are let through one after the other and no more than `max_failures` pass in a
/// window. The meaning of a counter is the one of the sign-in throttle: a window starts with its first
/// request, and the counter locks for one `window_seconds` once it has counted `max_failures`.
///
/// Callers must always give the counters in the same order, which keeps two such transactions from
/// waiting for each other.
pub(crate) async fn admit_all(
    pool: &PgPool,
    counters: &[(&str, &str, Bucket)],
) -> Result<bool, CoreError> {
    let mut tx = pool.begin().await?;
    // The database's clock, read once: the window and the lock are compared with the stored times.
    let now = sqlx::query_scalar!(r#"SELECT now() as "now!""#)
        .fetch_one(&mut *tx)
        .await?;
    // What each counter becomes if the request is let through.
    let mut next = Vec::with_capacity(counters.len());
    for &(scope, key, bucket) in counters {
        let hash = key_hash(key);
        // The row has to exist to be locked; a new one starts empty.
        sqlx::query!(
            "INSERT INTO penfii.auth_throttle (scope, key_hash, window_start, failures)
             VALUES ($1, $2, $3, 0) ON CONFLICT (scope, key_hash) DO NOTHING",
            scope,
            &hash,
            now,
        )
        .execute(&mut *tx)
        .await?;
        let row = sqlx::query!(
            "SELECT window_start, failures, locked_until FROM penfii.auth_throttle
             WHERE scope = $1 AND key_hash = $2 FOR UPDATE",
            scope,
            &hash,
        )
        .fetch_one(&mut *tx)
        .await?;
        let (window_start, failures, locked_until) =
            (row.window_start, row.failures, row.locked_until);
        if locked_until.is_some_and(|until| until > now) {
            // Dropping the transaction undoes the empty rows made above and counts nothing.
            return Ok(false);
        }
        let fresh = window_start < now - Duration::seconds(bucket.window_seconds);
        let counted = if fresh { 1 } else { failures + 1 };
        let locked_until = if counted >= bucket.max_failures {
            Some(now + Duration::seconds(bucket.window_seconds))
        } else {
            locked_until
        };
        next.push((scope, hash, fresh, counted, locked_until));
    }
    for (scope, hash, fresh, counted, locked_until) in next {
        sqlx::query!(
            "UPDATE penfii.auth_throttle
             SET failures = $3, window_start = CASE WHEN $4 THEN $5 ELSE window_start END,
                 locked_until = $6
             WHERE scope = $1 AND key_hash = $2",
            scope,
            &hash,
            counted,
            fresh,
            now,
            locked_until,
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(true)
}

pub(crate) async fn clear(pool: &PgPool, scope: &str, key: &str) -> Result<(), CoreError> {
    let hash = key_hash(key);
    sqlx::query!(
        "DELETE FROM penfii.auth_throttle WHERE scope = $1 AND key_hash = $2",
        scope,
        &hash,
    )
    .execute(pool)
    .await?;
    Ok(())
}
