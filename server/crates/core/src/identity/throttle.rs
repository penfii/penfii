//! Failed-attempt throttle for password proofs (sign-in and reauthentication). The key is the
//! submitted identifier, whether or not an account exists, so a locked response says nothing about
//! the account.

use chrono::{Duration, Utc};
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
    let locked_until: Option<Option<chrono::DateTime<Utc>>> = sqlx::query_scalar(
        "SELECT locked_until FROM penfii.auth_throttle WHERE scope = $1 AND key_hash = $2",
    )
    .bind(scope)
    .bind(key_hash(key))
    .fetch_optional(pool)
    .await?;
    if let Some(Some(until)) = locked_until {
        let remaining = (until - Utc::now()).num_seconds();
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
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO penfii.auth_throttle (scope, key_hash, window_start, failures)
         VALUES ($1, $2, $3, 1)
         ON CONFLICT (scope, key_hash) DO UPDATE SET
             failures = CASE WHEN penfii.auth_throttle.window_start < $4
                             THEN 1 ELSE penfii.auth_throttle.failures + 1 END,
             window_start = CASE WHEN penfii.auth_throttle.window_start < $4
                                 THEN $3 ELSE penfii.auth_throttle.window_start END,
             locked_until = CASE
                 WHEN penfii.auth_throttle.window_start >= $4
                      AND penfii.auth_throttle.failures + 1 >= $5
                 THEN $6 ELSE penfii.auth_throttle.locked_until END",
    )
    .bind(scope)
    .bind(key_hash(key))
    .bind(now)
    .bind(now - Duration::seconds(bucket.window_seconds))
    .bind(bucket.max_failures)
    .bind(now + Duration::seconds(bucket.window_seconds))
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn clear(pool: &PgPool, scope: &str, key: &str) -> Result<(), CoreError> {
    sqlx::query("DELETE FROM penfii.auth_throttle WHERE scope = $1 AND key_hash = $2")
        .bind(scope)
        .bind(key_hash(key))
        .execute(pool)
        .await?;
    Ok(())
}
