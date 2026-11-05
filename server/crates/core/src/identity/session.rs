//! A signed-in session as the rest of the system sees it, recent-authentication checks, and
//! session management. `require_recent_authentication` is the single check that replaces
//! "type the current password".

use chrono::{DateTime, Duration, Utc};
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
}

/// `PENFII_REAUTH_WINDOW_SECONDS`, default 600.
pub fn reauth_window_seconds() -> i64 {
    std::env::var("PENFII_REAUTH_WINDOW_SECONDS")
        .ok()
        .and_then(|raw| raw.trim().parse::<i64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_REAUTH_WINDOW_SECONDS)
}

/// Resolves a session token. Also keeps `last_seen_at` roughly current (at most once a minute).
pub async fn authenticate_session(pool: &PgPool, token: &str) -> Result<AuthSession, CoreError> {
    let row = sqlx::query(
        "SELECT s.id AS session_id, s.auth_method, s.authenticated_at, s.last_reauth_at,
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
    let last_seen: Option<DateTime<Utc>> = row.try_get("last_seen_at")?;
    if last_seen.is_none_or(|seen| Utc::now() - seen > Duration::seconds(60)) {
        sqlx::query("UPDATE penfii.sessions SET last_seen_at = now() WHERE id = $1")
            .bind(id)
            .execute(pool)
            .await?;
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

/// Ok when the session was signed in or reauthenticated within the window.
pub async fn require_recent_authentication(
    pool: &PgPool,
    session: &AuthSession,
) -> Result<(), CoreError> {
    let window = Duration::seconds(reauth_window_seconds());
    if session
        .last_reauth_at
        .is_some_and(|at| Utc::now() - at <= window)
    {
        return Ok(());
    }
    Err(CoreError::ReauthRequired {
        methods: reauth_methods(pool, session.user.id).await?,
    })
}

/// Checks the account password (throttled per account) without touching the session.
pub(crate) async fn verify_account_password(
    pool: &PgPool,
    user_id: Uuid,
    password: &str,
) -> Result<(), CoreError> {
    let scope = "reauth";
    let key = user_id.to_string();
    throttle::check(pool, scope, &key).await?;
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
            throttle::record_failure(pool, scope, &key).await?;
            Err(CoreError::Unauthenticated)
        }
        // No password credential: the caller reports the methods that do work.
        None => Err(CoreError::ReauthRequired {
            methods: reauth_methods(pool, user_id).await?,
        }),
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
        "SELECT id, auth_method, device_label, created_at, last_seen_at, last_reauth_at, expires_at
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
    let done = sqlx::query(
        "UPDATE penfii.sessions SET revoked_at = now()
         WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL",
    )
    .bind(session_id)
    .bind(user_id)
    .execute(pool)
    .await?;
    if done.rows_affected() == 0 {
        return Err(CoreError::NotFound);
    }
    Ok(())
}

/// Revokes every session of the user except `keep`; returns how many. Needs recent authentication.
pub async fn revoke_other_sessions(pool: &PgPool, session: &AuthSession) -> Result<u64, CoreError> {
    require_recent_authentication(pool, session).await?;
    let done = sqlx::query(
        "UPDATE penfii.sessions SET revoked_at = now()
         WHERE user_id = $1 AND id <> $2 AND revoked_at IS NULL",
    )
    .bind(session.user.id)
    .bind(session.id)
    .execute(pool)
    .await?;
    Ok(done.rows_affected())
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
