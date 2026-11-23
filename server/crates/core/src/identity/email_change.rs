//! Changing the account's e-mail address (FEA-140).
//!
//! The new address is not the account's until its owner has opened the link mailed to it. Until then
//! the account signs in and recovers with the address it has; a waiting request changes nothing about
//! it. Using the link makes the new address the account's *verified* address, closes the account's
//! other pending links (a reset link mailed to the old address must not outlive it) and tells the old
//! address. Starting a change needs a recent authentication (the same proof as any sensitive
//! action) and a server that can send mail: without SMTP there is no way to prove the new address, so
//! the self-service change is refused with `email_change_unavailable` and the operator's
//! `admin-cli user-set-email` is the way (it leaves the new address *unverified*).
//!
//! A token is 256 bits of randomness kept only as a hash, good for one use and for [`TOKEN_HOURS`]
//! hours; a newer request replaces the older one. Neither a token nor an address is audited.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use super::{AuthSession, normalize_email, throttle};
use crate::error::CoreError;
use crate::limits::Bucket;
use crate::mail::Mailer;

/// How long the link works.
const TOKEN_HOURS: i64 = 24;
/// Requests of one account per hour.
const PER_ACCOUNT: Bucket = Bucket {
    max_failures: 5,
    window_seconds: 3600,
};
/// Mails to one address per hour: the address belongs to somebody who may not want them.
const PER_ADDRESS: Bucket = Bucket {
    max_failures: 3,
    window_seconds: 3600,
};

/// The change that is waiting for its link to be opened.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingEmailChange {
    pub email: String,
    pub expires_at: DateTime<Utc>,
}

fn unavailable() -> CoreError {
    CoreError::conflict(
        "email_change_unavailable",
        "this server does not send e-mail, so the address cannot be changed here; ask the instance operator",
    )
}

fn invalid_token() -> CoreError {
    CoreError::conflict(
        "email_change_token_invalid",
        "this confirmation link is not valid any more",
    )
}

/// The waiting change of the account, if any.
pub async fn pending(
    pool: &PgPool,
    user_id: Uuid,
) -> Result<Option<PendingEmailChange>, CoreError> {
    let row: Option<(String, DateTime<Utc>)> = sqlx::query_as(
        "SELECT new_email, expires_at FROM penfii.email_change_requests
         WHERE user_id = $1 AND closed_at IS NULL AND expires_at > now()",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(email, expires_at)| PendingEmailChange { email, expires_at }))
}

/// Starts a change: mails a confirmation link to `new_email`. The account keeps its address.
pub async fn request(
    pool: &PgPool,
    mailer: Option<&Mailer>,
    session: &AuthSession,
    new_email: &str,
) -> Result<PendingEmailChange, CoreError> {
    let mailer = mailer.ok_or_else(unavailable)?;
    let (email, email_key) = normalize_email(new_email)?;
    super::require_recent_authentication(pool, session).await?;
    let user_id = session.user.id;
    let current: Option<(String, bool, Option<String>)> = sqlx::query_as(
        "SELECT email_key, email_verified_at IS NOT NULL, locale FROM penfii.users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    let (current_key, verified, locale) = current.ok_or(CoreError::NotFound)?;
    // The same address is a request only for an account whose address was never confirmed: that is
    // how an address an operator set becomes verified.
    if current_key == email_key && verified {
        return Err(CoreError::validation(
            "that is already the account's address",
        ));
    }
    // Counted before the address is looked up: whether an address has an account is only answered
    // within the account's limit, so the answer cannot be used to list who has one.
    // Always in this order (see `throttle::reserve`).
    let key = user_id.to_string();
    throttle::admit_or_limited(
        pool,
        &[
            ("email_change", key.as_str(), PER_ACCOUNT),
            ("email_change_to", email_key.as_str(), PER_ADDRESS),
        ],
    )
    .await?;
    if current_key != email_key {
        let taken: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM penfii.users WHERE email_key = $1 AND id <> $2)",
        )
        .bind(&email_key)
        .bind(user_id)
        .fetch_one(pool)
        .await?;
        if taken {
            return Err(CoreError::conflict(
                "account_exists",
                "account already exists",
            ));
        }
    }

    let (token, hash) = super::new_secret();
    let request_id = Uuid::new_v4();
    let ttl = (TOKEN_HOURS * 3600) as f64;
    let mut tx = pool.begin().await?;
    // Requests of one account that run together take turns: the second one replaces the first.
    sqlx::query("SELECT id FROM penfii.users WHERE id = $1 FOR UPDATE")
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?;
    sqlx::query(
        "DELETE FROM penfii.email_change_requests
         WHERE user_id = $1 OR expires_at < now() - interval '1 day'",
    )
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    let expires_at: DateTime<Utc> = sqlx::query_scalar(
        "INSERT INTO penfii.email_change_requests (id, user_id, new_email, new_email_key, token_hash, expires_at)
         VALUES ($1, $2, $3, $4, $5, now() + make_interval(secs => $6))
         RETURNING expires_at",
    )
    .bind(request_id)
    .bind(user_id)
    .bind(&email)
    .bind(&email_key)
    .bind(&hash)
    .bind(ttl)
    .fetch_one(&mut *tx)
    .await?;
    audit(&mut tx, user_id, "user.email_change_requested", "web").await?;
    tx.commit().await?;

    let locale = locale_or_default(pool, locale).await?;
    // The token travels in the fragment: a browser never sends it to the server, so it is in no
    // access log and no Referer header.
    let link = mailer.link(&format!("/confirm-email#{token}"));
    let (subject, body) = crate::mail::email_change_message(&locale, &link, TOKEN_HOURS);
    if let Err(reason) = mailer.send(&email, &subject, body).await {
        // The link that was never delivered is not left waiting.
        sqlx::query("DELETE FROM penfii.email_change_requests WHERE id = $1")
            .bind(request_id)
            .execute(pool)
            .await?;
        return Err(CoreError::conflict("email_not_sent", reason));
    }
    Ok(PendingEmailChange { email, expires_at })
}

/// Uses the link: the new address becomes the account's, verified. Every refusal of the token
/// (unknown, used, replaced, expired, account disabled) is the same `email_change_token_invalid`.
pub async fn confirm(
    pool: &PgPool,
    mailer: Option<Arc<Mailer>>,
    token: &str,
) -> Result<(), CoreError> {
    if token.is_empty() || token.len() > 256 {
        return Err(invalid_token());
    }
    let mut tx = pool.begin().await?;
    let row: Option<(Uuid, Uuid, String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT r.id, r.user_id, r.new_email, r.new_email_key, u.email, u.locale
         FROM penfii.email_change_requests r
         JOIN penfii.users u ON u.id = r.user_id
         WHERE r.token_hash = $1 AND r.closed_at IS NULL AND r.expires_at > now()
           AND u.kind = 'HUMAN' AND u.disabled_at IS NULL
         FOR UPDATE OF r, u",
    )
    .bind(super::token_hash(token))
    .fetch_optional(&mut *tx)
    .await?;
    let Some((request_id, user_id, new_email, new_key, old_email, locale)) = row else {
        return Err(invalid_token());
    };
    let changed = sqlx::query(
        "UPDATE penfii.users SET email = $2, email_key = $3, email_verified_at = now() WHERE id = $1",
    )
    .bind(user_id)
    .bind(&new_email)
    .bind(&new_key)
    .execute(&mut *tx)
    .await;
    if let Err(error) = changed {
        if super::is_unique(&error) {
            // Somebody else took the address after the request was made.
            return Err(CoreError::conflict(
                "account_exists",
                "account already exists",
            ));
        }
        return Err(error.into());
    }
    sqlx::query("UPDATE penfii.email_change_requests SET closed_at = now() WHERE id = $1")
        .bind(request_id)
        .execute(&mut *tx)
        .await?;
    // A reset link that was mailed to the old address must not outlive it.
    sqlx::query(
        "UPDATE penfii.password_reset_tokens SET closed_at = now()
         WHERE user_id = $1 AND closed_at IS NULL",
    )
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    audit(&mut tx, user_id, "user.email_changed", "email").await?;
    tx.commit().await?;

    // The old address is told, once the change stands; nobody waits for it.
    if let Some(mailer) = mailer
        && !old_email.eq_ignore_ascii_case(&new_email)
    {
        let pool = pool.clone();
        tokio::spawn(async move {
            let Ok(locale) = locale_or_default(&pool, locale).await else {
                return;
            };
            let (subject, body) = crate::mail::email_changed_notice(&locale);
            let _ = mailer.send(&old_email, &subject, body).await;
        });
    }
    Ok(())
}

/// `admin-cli user-set-email`: the operator sets the address of an account. It is not a proof that the
/// new address belongs to the person, so the account's address becomes *unverified* (it signs in with
/// it, but no password reset is mailed to it until the person confirms it from their account page).
/// Waiting changes and reset links are closed. Returns the account's id.
pub async fn operator_set_email(
    pool: &PgPool,
    current_email: &str,
    new_email: &str,
) -> Result<Uuid, CoreError> {
    let (_, current_key) = normalize_email(current_email)?;
    let (email, email_key) = normalize_email(new_email)?;
    let mut tx = pool.begin().await?;
    let user_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM penfii.users WHERE email_key = $1 AND kind = 'HUMAN' FOR UPDATE",
    )
    .bind(&current_key)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(CoreError::NotFound)?;
    if current_key == email_key {
        return Err(CoreError::validation(
            "the account already has that address",
        ));
    }
    let changed = sqlx::query(
        "UPDATE penfii.users SET email = $2, email_key = $3, email_verified_at = NULL WHERE id = $1",
    )
    .bind(user_id)
    .bind(&email)
    .bind(&email_key)
    .execute(&mut *tx)
    .await;
    if let Err(error) = changed {
        if super::is_unique(&error) {
            return Err(CoreError::conflict(
                "account_exists",
                "another account already has that address",
            ));
        }
        return Err(error.into());
    }
    sqlx::query(
        "UPDATE penfii.email_change_requests SET closed_at = now()
         WHERE user_id = $1 AND closed_at IS NULL",
    )
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE penfii.password_reset_tokens SET closed_at = now()
         WHERE user_id = $1 AND closed_at IS NULL",
    )
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    audit(&mut tx, user_id, "user.email_set", "admin-cli").await?;
    tx.commit().await?;
    Ok(user_id)
}

async fn locale_or_default(pool: &PgPool, locale: Option<String>) -> Result<String, CoreError> {
    match locale.filter(|value| !value.is_empty()) {
        Some(locale) => Ok(locale),
        None => Ok(
            sqlx::query_scalar("SELECT default_locale FROM penfii.instance_settings")
                .fetch_optional(pool)
                .await?
                .unwrap_or_else(|| "ko-KR".to_string()),
        ),
    }
}

/// A user-level entry of the audit log: the account, and how it was done. No token, no address.
async fn audit(
    conn: &mut sqlx::PgConnection,
    user_id: Uuid,
    action: &str,
    via: &str,
) -> Result<(), CoreError> {
    sqlx::query(
        "INSERT INTO penfii.audit_entries
            (id, actor_user_id, action_code, outcome_code, target_kind, target_id, data)
         VALUES ($1, $5, $3, 'OK', 'user', $2, jsonb_build_object('via', $4::text))",
    )
    .bind(Uuid::new_v4())
    .bind(user_id)
    .bind(action)
    .bind(via)
    .bind((via != "admin-cli").then_some(user_id))
    .execute(conn)
    .await?;
    Ok(())
}
