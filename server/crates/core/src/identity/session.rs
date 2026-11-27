//! A signed-in session as the rest of the system sees it, recent-authentication checks, and
//! session management. `require_recent_authentication` is the single check that replaces
//! "type the current password".

use chrono::{DateTime, Utc};
use penfii_contracts::User;
use serde::Serialize;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use super::{UserRow, throttle, token_hash, user_from_row, verify_password};
use crate::error::CoreError;
use crate::limits::ThrottleLimits;

const DEFAULT_REAUTH_WINDOW_SECONDS: i64 = 600;

#[derive(Debug, Clone)]
pub struct AuthSession {
    pub id: Uuid,
    pub user: User,
    pub auth_method: Option<String>,
    pub authenticated_at: Option<DateTime<Utc>>,
    pub last_reauth_at: Option<DateTime<Utc>>,
    /// How long ago `last_reauth_at` was, by the database's clock when the session was read. The
    /// reauthentication window is judged with this, not with the clock of this process.
    pub reauth_age_seconds: Option<f64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub id: Uuid,
    pub current: bool,
    pub auth_method: Option<String>,
    pub device_label: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub last_reauth_at: Option<DateTime<Utc>>,
    pub expires_at: DateTime<Utc>,
    /// A session the person chose to keep (see `LongSession`): `expires_at` is then the earlier of
    /// the inactivity end and `absolute_expires_at`.
    pub long_lived: bool,
    pub absolute_expires_at: Option<DateTime<Utc>>,
}

/// `PENFII_REAUTH_WINDOW_SECONDS`, default 600.
pub fn reauth_window_seconds() -> i64 {
    std::env::var("PENFII_REAUTH_WINDOW_SECONDS")
        .ok()
        .and_then(|raw| raw.trim().parse::<i64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_REAUTH_WINDOW_SECONDS)
}

/// Records activity on a session: `last_seen_at`, and for a long session its inactivity expiry,
/// moved to `now + idle_seconds` but never past `absolute_expires_at`, which activity cannot move.
/// An ordinary session keeps its fixed `expires_at`. Only a live session is touched, so an expired
/// or revoked one is never brought back. Requests an authenticated person makes are activity; a
/// socket's periodic check of its session is not (see `identity::session_is_live`).
pub(crate) async fn touch_session(pool: &PgPool, session_id: Uuid) -> Result<(), CoreError> {
    sqlx::query(
        "UPDATE penfii.sessions
         SET last_seen_at = now(),
             expires_at = CASE WHEN absolute_expires_at IS NULL THEN expires_at
                               ELSE LEAST(absolute_expires_at, now() + make_interval(secs => idle_seconds)) END
         WHERE id = $1 AND revoked_at IS NULL AND expires_at > now()",
    )
    .bind(session_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Applies the operator's current long-session settings to the long sessions that are still live,
/// once at start (the settings are environment variables, so they change only with a restart):
///
/// - the feature is off: every live long session becomes an ordinary one that ends at the ordinary
///   lifetime counted from when it was issued (or earlier, as it was); nothing is signed out that
///   an ordinary session would still hold, and nothing stays long;
/// - a shorter absolute lifetime or inactivity window: the session is cut to the new bounds
///   counted from when it was issued and last seen (a session already past them ends);
/// - a longer one: nothing changes for the sessions that exist; they keep the end they were issued
///   with (the absolute end is a promise made at sign-in), new sessions get the new values.
///
/// Settings can therefore only shorten or end a session that exists, never extend it. Returns how
/// many sessions it changed.
pub async fn apply_long_session_policy(
    pool: &PgPool,
    limits: &crate::Limits,
) -> Result<u64, CoreError> {
    let long = limits.long_session;
    let changed = if !long.enabled {
        let Some(ttl) = limits.session_ttl_seconds else {
            return Ok(0);
        };
        sqlx::query(
            "UPDATE penfii.sessions
             SET expires_at = LEAST(expires_at, created_at + make_interval(secs => $1)),
                 absolute_expires_at = NULL, idle_seconds = NULL
             WHERE absolute_expires_at IS NOT NULL AND revoked_at IS NULL AND expires_at > now()",
        )
        .bind(ttl as f64)
        .execute(pool)
        .await?
        .rows_affected()
    } else {
        sqlx::query(
            "UPDATE penfii.sessions
             SET absolute_expires_at = LEAST(absolute_expires_at, created_at + make_interval(secs => $1)),
                 idle_seconds = LEAST(idle_seconds, $2::int),
                 expires_at = LEAST(expires_at,
                                    created_at + make_interval(secs => $1),
                                    COALESCE(last_seen_at, created_at)
                                        + make_interval(secs => LEAST(idle_seconds, $2::int)))
             WHERE absolute_expires_at IS NOT NULL AND revoked_at IS NULL AND expires_at > now()
               AND (absolute_expires_at > created_at + make_interval(secs => $1)
                    OR idle_seconds > $2::int)",
        )
        .bind(long.max_age_seconds as f64)
        .bind(long.effective_idle_seconds() as i32)
        .execute(pool)
        .await?
        .rows_affected()
    };
    Ok(changed)
}

/// Resolves a session token. Also keeps `last_seen_at` roughly current (at most once a minute).
pub async fn authenticate_session(pool: &PgPool, token: &str) -> Result<AuthSession, CoreError> {
    authenticate_session_with(pool, token, true).await
}

/// [`authenticate_session`] for background traffic: nothing is recorded, so it cannot keep a long session alive.
pub async fn authenticate_session_background(
    pool: &PgPool,
    token: &str,
) -> Result<AuthSession, CoreError> {
    authenticate_session_with(pool, token, false).await
}

async fn authenticate_session_with(
    pool: &PgPool,
    token: &str,
    activity: bool,
) -> Result<AuthSession, CoreError> {
    let row = sqlx::query(
        "SELECT s.id AS session_id, s.auth_method, s.authenticated_at, s.last_reauth_at,
                EXTRACT(EPOCH FROM (now() - s.last_reauth_at))::float8 AS reauth_age_seconds,
                EXTRACT(EPOCH FROM (now() - s.last_seen_at))::float8 AS seen_age_seconds,
                s.last_seen_at,
                u.id, u.email, u.display_name, u.locale, u.time_zone, u.email_verified_at,
                u.job_title, u.status_emoji, u.status_text, u.status_clear_at,
                (u.avatar_object_key IS NOT NULL) AS has_avatar
         FROM penfii.sessions s
         JOIN penfii.users u ON u.id = s.user_id
         WHERE s.token_hash = $1
           AND s.revoked_at IS NULL
           AND s.expires_at > now()
           AND u.disabled_at IS NULL",
    )
    .bind(token_hash(token))
    .fetch_optional(pool)
    .await?
    .ok_or(CoreError::Unauthenticated)?;
    let id: Uuid = row.try_get("session_id")?;
    let seen_age: Option<f64> = row.try_get("seen_age_seconds")?;
    if activity && seen_age.is_none_or(|age| age > 60.0) {
        touch_session(pool, id).await?;
    }
    let user = user_from_row(UserRow {
        id: row.try_get("id")?,
        email: row.try_get("email")?,
        display_name: row.try_get("display_name")?,
        locale: row.try_get("locale")?,
        time_zone: row.try_get("time_zone")?,
        email_verified_at: row.try_get("email_verified_at")?,
        job_title: row.try_get("job_title")?,
        status_emoji: row.try_get("status_emoji")?,
        status_text: row.try_get("status_text")?,
        status_clear_at: row.try_get("status_clear_at")?,
        has_avatar: row.try_get("has_avatar")?,
    });
    Ok(AuthSession {
        id,
        user,
        auth_method: row.try_get("auth_method")?,
        authenticated_at: row.try_get("authenticated_at")?,
        last_reauth_at: row.try_get("last_reauth_at")?,
        reauth_age_seconds: row.try_get("reauth_age_seconds")?,
    })
}

/// Methods this account can use to prove itself again.
pub async fn reauth_methods(pool: &PgPool, user_id: Uuid) -> Result<Vec<String>, CoreError> {
    let mut methods = Vec::new();
    let has_password: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM penfii.password_credentials WHERE user_id = $1)",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await?;
    if has_password {
        methods.push("password".to_string());
    }
    let providers: Vec<String> = sqlx::query_scalar(
        "SELECT p.provider_key FROM penfii.external_identities i
         JOIN penfii.auth_providers p ON p.id = i.provider_id
         WHERE i.user_id = $1 AND p.enabled AND p.provider_key IS NOT NULL
         ORDER BY p.provider_key",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    methods.extend(providers.into_iter().map(|key| format!("provider:{key}")));
    Ok(methods)
}

/// Whether the session was signed in or reauthenticated within the window. No query: a caller that
/// holds a transaction asks this, and only when it is `false` lets go of its connection before it
/// calls [`reauth_required`].
pub(crate) fn recently_authenticated(session: &AuthSession) -> bool {
    let window = reauth_window_seconds() as f64;
    session.reauth_age_seconds.is_some_and(|age| age <= window)
}

/// The refusal for a session that is not recently authenticated, with the ways it can authenticate
/// again. It reads on the pool, so it is not called while a transaction is open.
pub(crate) async fn reauth_required(pool: &PgPool, session: &AuthSession) -> CoreError {
    match reauth_methods(pool, session.user.id).await {
        Ok(methods) => CoreError::ReauthRequired { methods },
        Err(error) => error,
    }
}

/// Ok when the session was signed in or reauthenticated within the window.
pub async fn require_recent_authentication(
    pool: &PgPool,
    session: &AuthSession,
) -> Result<(), CoreError> {
    if recently_authenticated(session) {
        return Ok(());
    }
    Err(reauth_required(pool, session).await)
}

/// Checks the account password (throttled per account) without touching the session.
pub(crate) async fn verify_account_password(
    pool: &PgPool,
    user_id: Uuid,
    password: &str,
) -> Result<(), CoreError> {
    let scope = "reauth";
    let key = user_id.to_string();
    // Reserve, verify, settle (see `throttle`): the attempt is counted before the hash is checked.
    let counters = [(scope, key.as_str(), throttle::REAUTH)];
    throttle::admit_or_limited(pool, &counters).await?;
    let hash: Option<String> = sqlx::query_scalar(
        "SELECT password_hash FROM penfii.password_credentials WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    let valid = match &hash {
        Some(hash) => verify_password(password, hash).await,
        None => false,
    };
    match hash {
        Some(_) if valid => throttle::clear(pool, scope, &key).await,
        Some(_) => {
            super::audit_lock_reached(pool, user_id, "user.reauth_locked", &[(scope, &key)]).await;
            Err(CoreError::Unauthenticated)
        }
        // No password credential: nothing was guessed, so the attempt is not held against the account;
        // the caller reports the methods that do work.
        None => {
            throttle::refund(pool, scope, &key, throttle::REAUTH).await?;
            Err(CoreError::ReauthRequired {
                methods: reauth_methods(pool, user_id).await?,
            })
        }
    }
}

pub(crate) async fn mark_reauthenticated(
    pool: &PgPool,
    session_id: Uuid,
    method: &str,
) -> Result<(), CoreError> {
    sqlx::query(
        "UPDATE penfii.sessions SET last_reauth_at = now(), last_reauth_method = $2 WHERE id = $1",
    )
    .bind(session_id)
    .bind(method)
    .execute(pool)
    .await?;
    Ok(())
}

/// `POST /v1/me/reauth/password`: proves the signed-in person with their password.
pub async fn reauth_with_password(
    pool: &PgPool,
    session: &AuthSession,
    password: &str,
) -> Result<(), CoreError> {
    super::require_password(password)?;
    verify_account_password(pool, session.user.id, password).await?;
    mark_reauthenticated(pool, session.id, "password").await
}

pub async fn list_sessions(
    pool: &PgPool,
    user_id: Uuid,
    current: Uuid,
) -> Result<Vec<SessionInfo>, CoreError> {
    let rows = sqlx::query(
        "SELECT id, auth_method, device_label, created_at, last_seen_at, last_reauth_at, expires_at,
                absolute_expires_at
         FROM penfii.sessions
         WHERE user_id = $1 AND revoked_at IS NULL AND expires_at > now()
         ORDER BY created_at DESC, id",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|row| {
            let id: Uuid = row.try_get("id")?;
            Ok(SessionInfo {
                id,
                current: id == current,
                auth_method: row.try_get("auth_method")?,
                device_label: row.try_get("device_label")?,
                created_at: row.try_get("created_at")?,
                last_seen_at: row.try_get("last_seen_at")?,
                last_reauth_at: row.try_get("last_reauth_at")?,
                expires_at: row.try_get("expires_at")?,
                absolute_expires_at: row.try_get("absolute_expires_at")?,
                long_lived: row
                    .try_get::<Option<DateTime<Utc>>, _>("absolute_expires_at")?
                    .is_some(),
            })
        })
        .collect::<Result<_, sqlx::Error>>()
        .map_err(Into::into)
}

/// Revokes one of the caller's own sessions. Another person's session id is `NotFound`.
pub async fn revoke_session(
    pool: &PgPool,
    user_id: Uuid,
    session_id: Uuid,
) -> Result<(), CoreError> {
    let mut tx = pool.begin().await?;
    let done = sqlx::query(
        "UPDATE penfii.sessions SET revoked_at = now()
         WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL",
    )
    .bind(session_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    if done.rows_affected() == 0 {
        return Err(CoreError::NotFound);
    }
    super::audit_user(
        &mut tx,
        user_id,
        Some(user_id),
        "user.session_revoked",
        "OK",
        serde_json::json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Ends the web sessions of the user, except `keep` when it is given, and returns how many. The one place
/// that says what "end sessions" is, for every action that does it (other devices, a changed password,
/// MFA turned on or off, a reset, an operator); OAuth/MCP grants are not touched here (see
/// `admin::end_all_access` for the action that ends those as well).
pub(crate) async fn end_sessions(
    conn: &mut sqlx::PgConnection,
    user_id: Uuid,
    keep: Option<Uuid>,
) -> Result<u64, CoreError> {
    let done = sqlx::query(
        "UPDATE penfii.sessions SET revoked_at = now()
         WHERE user_id = $1 AND revoked_at IS NULL AND ($2::uuid IS NULL OR id <> $2)",
    )
    .bind(user_id)
    .bind(keep)
    .execute(conn)
    .await?;
    Ok(done.rows_affected())
}

/// Every web session of the user except `keep` (this device).
pub(crate) async fn end_other_sessions(
    conn: &mut sqlx::PgConnection,
    user_id: Uuid,
    keep: Uuid,
) -> Result<u64, CoreError> {
    end_sessions(conn, user_id, Some(keep)).await
}

/// Revokes every session of the user except the current one; returns how many. Needs recent
/// authentication. Connected apps (MCP) stay.
pub async fn revoke_other_sessions(pool: &PgPool, session: &AuthSession) -> Result<u64, CoreError> {
    require_recent_authentication(pool, session).await?;
    let mut tx = pool.begin().await?;
    let count = end_other_sessions(&mut tx, session.user.id, session.id).await?;
    if count > 0 {
        super::audit_user(
            &mut tx,
            session.user.id,
            Some(session.user.id),
            "user.sessions_revoked",
            "OK",
            serde_json::json!({"count": count}),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(count)
}

/// Per-address throttle: shared addresses are common, so the limit is higher than the
/// per-account one. `scope` names the action; the address is `None` when it is not known (tests,
/// direct calls), which means no throttling by address.
pub async fn check_address(
    pool: &PgPool,
    scope: &str,
    address: Option<&str>,
) -> Result<(), CoreError> {
    match address {
        Some(address) => throttle::check(pool, &format!("{scope}_ip"), address).await,
        None => Ok(()),
    }
}

pub async fn record_address_failure(
    pool: &PgPool,
    scope: &str,
    address: Option<&str>,
    limits: &ThrottleLimits,
) -> Result<(), CoreError> {
    match address {
        Some(address) => {
            throttle::record_failure_limit(pool, &format!("{scope}_ip"), address, limits.address)
                .await
        }
        None => Ok(()),
    }
}
