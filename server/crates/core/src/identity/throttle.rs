//! Failed-attempt throttle for password proofs (sign-in and reauthentication). The key is the
//! submitted identifier, whether or not an account exists, so a locked response says nothing about
//! the account.
//!
//! A proof that is checked with a slow hash (a password, later a one-time code) uses the
//! **reserve, verify, settle** pattern, which no number of parallel requests can get around:
//!
//! 1. [`admit_or_limited`] counts the attempt in every counter and refuses it (`RateLimited`) when
//!    any of them is full. Counting and testing are one locked transaction, so requests that arrive
//!    together are let in one after the other and at most `max_failures` of them pass per window.
//! 2. The proof is checked (outside any transaction).
//! 3. On success the caller [`clear`]s the counters of the account and [`refund`]s the shared ones
//!    (an address is not charged for a right answer). On failure nothing is left to do: the attempt
//!    is already counted.
//!
//! The older [`check`] followed by [`record_failure_limit`] is not atomic and stays only for
//! callers that have nothing slow in between.

use chrono::Duration;
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use crate::error::CoreError;
use crate::limits::Bucket;

/// Re-authentication of a signed-in person: 5 failures in 15 minutes, per account.
pub(crate) const REAUTH: Bucket = Bucket {
    max_failures: 5,
    window_seconds: 15 * 60,
};

/// Codes of a second factor: 10 wrong ones in 15 minutes lock the counter, per account. Used for the
/// code step of a sign-in (keyed by the e-mail, `mfa_login`) and for the actions of the signed-in person
/// (keyed by the account, `mfa`).
pub(crate) const MFA: Bucket = Bucket {
    max_failures: 10,
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
    Ok(matches!(
        reserve(pool, counters).await?,
        Admission::Admitted
    ))
}

/// What [`reserve`] decided.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Admission {
    Admitted,
    /// A counter is full; the attempt is not counted. The time is the database's, in whole seconds.
    Locked {
        retry_after_seconds: i64,
    },
}

/// Step 1 of reserve, verify, settle (see the module comment): [`admit_all`] for an attempt that
/// must be answered with `RateLimited` and its retry time.
pub(crate) async fn admit_or_limited(
    pool: &PgPool,
    counters: &[(&str, &str, Bucket)],
) -> Result<(), CoreError> {
    match reserve(pool, counters).await? {
        Admission::Admitted => Ok(()),
        Admission::Locked {
            retry_after_seconds,
        } => Err(CoreError::RateLimited {
            retry_after_seconds,
        }),
    }
}

/// Takes one attempt back from a counter after a right answer, and lifts the lock that attempt
/// itself may have set. Used for counters that are shared by many people (an address): the attempt
/// was counted before it was known to be wrong.
pub(crate) async fn refund(
    pool: &PgPool,
    scope: &str,
    key: &str,
    bucket: Bucket,
) -> Result<(), CoreError> {
    let hash = key_hash(key);
    sqlx::query(
        "UPDATE penfii.auth_throttle
         SET failures = GREATEST(failures - 1, 0),
             locked_until = CASE WHEN failures - 1 < $3 THEN NULL ELSE locked_until END
         WHERE scope = $1 AND key_hash = $2",
    )
    .bind(scope)
    .bind(&hash)
    .bind(bucket.max_failures)
    .execute(pool)
    .await?;
    Ok(())
}

/// [`admit_all`] with the retry time of a refusal.
pub(crate) async fn reserve(
    pool: &PgPool,
    counters: &[(&str, &str, Bucket)],
) -> Result<Admission, CoreError> {
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
        if let Some(until) = locked_until.filter(|until| *until > now) {
            // Dropping the transaction undoes the empty rows made above and counts nothing.
            return Ok(Admission::Locked {
                retry_after_seconds: (until - now).num_seconds().max(1),
            });
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
    Ok(Admission::Admitted)
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
