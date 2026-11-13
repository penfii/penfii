//! Password reset by e-mail (FEA-063).
//!
//! The request is anonymous, so it must not tell whether an account exists: not by what it answers
//! and not by how long it takes. The handler therefore does the same work for every well-formed
//! request (count it, then hand the rest to a background task) and answers at once; whether a mail is
//! sent is decided in the task, where nobody can observe it. A request for an unknown, disabled or
//! password-less account (one that only signs in through an external provider) ends there silently.
//!
//! A reset token is 256 bits of randomness, kept only as a hash, good for one use and for
//! [`TOKEN_MINUTES`] minutes. A newer request replaces the older token. Using one sets the new
//! password, ends every session of the account and is written to the audit log; neither the token nor
//! a password is ever logged or audited.

use sqlx::PgPool;
use uuid::Uuid;

use super::{account_address_key, throttle};
use crate::error::CoreError;
use crate::limits::ThrottleLimits;
use crate::mail::Mailer;

/// How long a reset link works.
const TOKEN_MINUTES: i64 = 30;

/// Counts a reset request against the reset counters and says whether it is let through: the
/// counters are separate from the sign-in ones but have the same three keys and the same limits
/// (see `ThrottleLimits`). Every request counts, not only failures, and a counter that is full lets
/// nothing through until its window is over. The answer to the caller is the same either way.
///
/// The three counters are tested and counted together in one transaction (`throttle::admit_all`), so
/// requests that arrive at the same moment cannot all slip past a counter that has room for one more.
pub async fn admit_request(
    pool: &PgPool,
    limits: &ThrottleLimits,
    email_key: &str,
    address: Option<&str>,
) -> Result<bool, CoreError> {
    let pair_key = account_address_key(email_key, address);
    // Always in this order (see `admit_all`); without an address only the account counters apply.
    let mut counters = Vec::with_capacity(3);
    if let Some(address) = address {
        counters.push(("reset_ip", address, limits.address));
    }
    counters.push(("reset", email_key, limits.account));
    counters.push((
        "reset_account_ip",
        pair_key.as_str(),
        limits.account_address,
    ));
    throttle::admit_all(pool, &counters).await
}

/// The part of a request that depends on the account. It runs in a background task: failures are
/// logged without the address or the token and are not told to anyone.
pub async fn process_request(pool: &PgPool, mailer: &Mailer, email_key: &str) {
    if let Err(error) = try_process(pool, mailer, email_key).await {
        tracing::warn!(?error, "a password reset request was not carried out");
    }
}

async fn try_process(pool: &PgPool, mailer: &Mailer, email_key: &str) -> Result<(), CoreError> {
    let account = sqlx::query!(
        "SELECT u.id, u.email, u.locale
         FROM penfii.users u
         JOIN penfii.password_credentials p ON p.user_id = u.id
         WHERE u.email_key = $1 AND u.kind = 'HUMAN' AND u.disabled_at IS NULL",
        email_key
    )
    .fetch_optional(pool)
    .await?;
    let Some(account) = account else {
        return Ok(());
    };
    let (user_id, email, locale) = (account.id, account.email, account.locale);
    let (token, hash) = super::new_secret();
    let mut tx = pool.begin().await?;
    // Requests for one account that run together take turns here, so the second one finds the first
    // one's token and replaces it: the account ends with one live token (the database also refuses a
    // second one, see the migration).
    sqlx::query!(
        "SELECT id FROM penfii.users WHERE id = $1 FOR UPDATE",
        user_id
    )
    .fetch_one(&mut *tx)
    .await?;
    // One live token per account: the new one replaces the old one. Tokens that are long over go too.
    sqlx::query!(
        "DELETE FROM penfii.password_reset_tokens
         WHERE user_id = $1 OR expires_at < now() - interval '1 day'",
        user_id
    )
    .execute(&mut *tx)
    .await?;
    let token_id = Uuid::new_v4();
    let ttl = (TOKEN_MINUTES * 60) as f64;
    sqlx::query!(
        "INSERT INTO penfii.password_reset_tokens (id, user_id, token_hash, expires_at)
         VALUES ($1, $2, $3, now() + make_interval(secs => $4))",
        token_id,
        user_id,
        &hash,
        ttl,
    )
    .execute(&mut *tx)
    .await?;
    audit(&mut tx, user_id, "user.password_reset_requested").await?;
    tx.commit().await?;

    let locale = match locale.filter(|value| !value.is_empty()) {
        Some(locale) => locale,
        None => sqlx::query_scalar!("SELECT default_locale FROM penfii.instance_settings")
            .fetch_optional(pool)
            .await?
            .unwrap_or_else(|| "ko-KR".to_string()),
    };
    // The token travels in the fragment (`#...`): a browser never sends it to the server, so it is
    // in no access log and in no Referer header. The page reads it and removes it from the address.
    let link = mailer.link(&format!("/reset-password#{token}"));
    let (subject, body) = crate::mail::password_reset_message(&locale, &link, TOKEN_MINUTES);
    // `send` logs why a mail failed and answers with a phrase; nobody waits for this task.
    let _ = mailer.send(&email, &subject, body).await;
    Ok(())
}

/// Uses a reset token to set a new password. Every refusal of the token (unknown, used, replaced,
/// expired, account disabled) is the same `reset_token_invalid`. A password that does not meet the
/// rule is refused without using the token up, so the person can try again with the same link.
pub async fn confirm(pool: &PgPool, token: &str, password: &str) -> Result<(), CoreError> {
    let invalid = || {
        CoreError::conflict(
            "reset_token_invalid",
            "this reset link is not valid any more",
        )
    };
    if token.is_empty() || token.len() > 256 {
        return Err(invalid());
    }
    // The token is looked at first without a lock, only to know whose password this is: a password
    // is checked and hashed (Argon2) before the transaction opens, so no connection or row lock is
    // held while it runs. The transaction then takes the token's row and decides.
    let token_hash = super::token_hash(token);
    let peek = sqlx::query_scalar!(
        "SELECT u.email
         FROM penfii.password_reset_tokens t
         JOIN penfii.users u ON u.id = t.user_id
         WHERE t.token_hash = $1
           AND t.closed_at IS NULL
           AND t.expires_at > now()
           AND u.kind = 'HUMAN' AND u.disabled_at IS NULL",
        &token_hash
    )
    .fetch_optional(pool)
    .await?;
    let Some(peek_email) = peek else {
        return Err(invalid());
    };
    super::validate_new_password(password, &peek_email)?;
    let password_hash = super::hash_password(password).await?;
    let mut tx = pool.begin().await?;
    let row: Option<(Uuid, Uuid, String)> = sqlx::query_as(
        "SELECT t.id, t.user_id, u.email
         FROM penfii.password_reset_tokens t
         JOIN penfii.users u ON u.id = t.user_id
         WHERE t.token_hash = $1
           AND t.closed_at IS NULL
           AND t.expires_at > now()
           AND u.kind = 'HUMAN' AND u.disabled_at IS NULL
         FOR UPDATE OF t",
    )
    .bind(super::token_hash(token))
    .fetch_optional(&mut *tx)
    .await?;
    let Some((token_id, user_id, email)) = row else {
        return Err(invalid());
    };
    // The e-mail could have been changed between the two reads; the rule is checked again (cheap).
    super::validate_new_password(password, &email)?;
    sqlx::query(
        "INSERT INTO penfii.password_credentials (user_id, password_hash) VALUES ($1, $2)
         ON CONFLICT (user_id) DO UPDATE SET password_hash = $2, changed_at = now()",
    )
    .bind(user_id)
    .bind(&password_hash)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE penfii.password_reset_tokens SET closed_at = now() WHERE id = $1")
        .bind(token_id)
        .execute(&mut *tx)
        .await?;
    // The password changed, so whoever held a session (the person, or someone who had got in) is
    // signed out everywhere.
    sqlx::query(
        "UPDATE penfii.sessions SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL",
    )
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    audit(&mut tx, user_id, "user.password_reset").await?;
    tx.commit().await?;
    Ok(())
}

/// A user-level entry (no Workspace) of the audit log. It names the account and nothing else: no
/// token, no password, no address of the requester.
async fn audit(
    conn: &mut sqlx::PgConnection,
    user_id: Uuid,
    action: &str,
) -> Result<(), CoreError> {
    sqlx::query(
        "INSERT INTO penfii.audit_entries
            (id, actor_user_id, action_code, outcome_code, target_kind, target_id, data)
         VALUES ($1, NULL, $2, 'OK', 'user', $3, '{\"via\":\"email\"}')",
    )
    .bind(Uuid::new_v4())
    .bind(action)
    .bind(user_id)
    .execute(conn)
    .await?;
    Ok(())
}
