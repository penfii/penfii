//! Optional TOTP two-step sign-in with single-use recovery codes (FEA-150).
//!
//! What is protected: signing in with the account's own password. An external provider sign-in is the
//! provider's own authentication (its policy decides), and a recent re-authentication for an ordinary
//! sensitive action stays "password or provider". Only the actions that lower the protection itself,
//! turning MFA off and issuing new recovery codes, ask for a recent re-authentication *and* a current
//! authenticator or recovery code.
//!
//! Secrets: the authenticator secret is stored encrypted with AES-256-GCM. The key is
//! `PENFII_MFA_ENCRYPTION_KEY` (64 hex characters) when it is set, and otherwise a random key kept in
//! `penfii.mfa_instance_key`, so enabling MFA needs no new setting. Both can exist: a secret names the
//! key that sealed it (`key_id`), and new secrets use the environment key when there is one. Recovery
//! codes are 60 random bits, stored only as SHA-256 hashes.
//!
//! Every code that is checked is counted first (see `throttle`): reserve, verify, settle.

use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use rand::RngCore;
use serde::Serialize;
use sha1::Sha1;
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool, Row};
use uuid::Uuid;

use super::{AuthSession, require_recent_authentication, session, throttle};
use crate::error::CoreError;

/// Authenticator apps agree on these (RFC 6238 defaults): SHA-1, 6 digits, 30 seconds.
const STEP_SECONDS: i64 = 30;
const DIGITS: u32 = 6;
/// A code of the previous or the next step is accepted too (clock drift, typing time).
const SKEW_STEPS: i64 = 1;
const SECRET_BYTES: usize = 20;
const RECOVERY_CODE_COUNT: usize = 10;
const RECOVERY_CODE_CHARS: usize = 12;
const RECOVERY_ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";
const ISSUER: &str = "Penfii";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MfaStatus {
    pub enabled: bool,
    pub enabled_at: Option<DateTime<Utc>>,
    /// Recovery codes that are still unused (`0` while MFA is off).
    pub recovery_codes_remaining: i64,
    /// A setup was started and not confirmed; it protects nothing yet.
    pub setup_pending: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MfaSetup {
    /// The shared secret as the grouped Base32 text an authenticator app accepts.
    pub secret: String,
    pub otpauth_uri: String,
}

// ---- TOTP (RFC 6238) ---------------------------------------------------------------------------

fn hotp(secret: &[u8], counter: u64, digits: u32) -> u32 {
    let mut mac = <Hmac<Sha1> as Mac>::new_from_slice(secret).expect("HMAC takes any key length");
    mac.update(&counter.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = (digest[digest.len() - 1] & 0x0f) as usize;
    let truncated = u32::from_be_bytes([
        digest[offset] & 0x7f,
        digest[offset + 1],
        digest[offset + 2],
        digest[offset + 3],
    ]);
    truncated % 10u32.pow(digits)
}

fn step_at(unix_seconds: i64) -> i64 {
    unix_seconds.div_euclid(STEP_SECONDS)
}

/// The accepted step a six-digit `code` belongs to around `now_step`, newest first, when there is one
/// after `after_step` (the newest step already used).
fn matching_step(secret: &[u8], code: u32, now_step: i64, after_step: i64) -> Option<i64> {
    let mut found = None;
    // Every candidate is computed, so the work does not depend on where (or whether) one matches.
    for step in (now_step - SKEW_STEPS..=now_step + SKEW_STEPS).rev() {
        if step <= 0 {
            continue;
        }
        let matches = hotp(secret, step as u64, DIGITS) == code;
        if matches && step > after_step && found.is_none() {
            found = Some(step);
        }
    }
    found
}

const BASE32: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

fn base32(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(5) * 8);
    let (mut buffer, mut bits) = (0u32, 0u32);
    for byte in bytes {
        buffer = (buffer << 8) | u32::from(*byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(BASE32[((buffer >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(BASE32[((buffer << (5 - bits)) & 31) as usize] as char);
    }
    out
}

fn otpauth_uri(account: &str, secret: &[u8]) -> String {
    let label = percent_encode(&format!("{ISSUER}:{account}"));
    format!(
        "otpauth://totp/{label}?secret={}&issuer={ISSUER}&algorithm=SHA1&digits={DIGITS}&period={STEP_SECONDS}",
        base32(secret)
    )
}

fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn grouped(text: &str) -> String {
    text.as_bytes()
        .chunks(4)
        .map(|chunk| std::str::from_utf8(chunk).unwrap_or_default())
        .collect::<Vec<_>>()
        .join(" ")
}

// ---- Keys and sealed secrets -------------------------------------------------------------------

fn key_id(key: &[u8; 32]) -> Vec<u8> {
    Sha256::digest(key)[..8].to_vec()
}

fn seal(key: &[u8; 32], user_id: Uuid, secret: &[u8]) -> Result<(Vec<u8>, Vec<u8>), CoreError> {
    let cipher =
        Aes256Gcm::new_from_slice(key).map_err(|error| CoreError::Internal(error.to_string()))?;
    let mut nonce = [0u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let sealed = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: secret,
                aad: user_id.as_bytes(),
            },
        )
        .map_err(|error| CoreError::Internal(error.to_string()))?;
    Ok((nonce.to_vec(), sealed))
}

fn open(key: &[u8; 32], user_id: Uuid, nonce: &[u8], sealed: &[u8]) -> Option<Vec<u8>> {
    if nonce.len() != 12 {
        return None;
    }
    Aes256Gcm::new_from_slice(key)
        .ok()?
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: sealed,
                aad: user_id.as_bytes(),
            },
        )
        .ok()
}

/// `PENFII_MFA_ENCRYPTION_KEY`: 32 bytes as 64 hex characters. Not set: `None` (the key kept in the
/// database is used); set but unreadable: an error (checked once at the start, see [`crate::config`]).
pub fn env_key(lookup: crate::config::Lookup<'_>) -> Result<Option<[u8; 32]>, String> {
    let Some(raw) = crate::config::get(lookup, "PENFII_MFA_ENCRYPTION_KEY") else {
        return Ok(None);
    };
    let bad = || {
        "PENFII_MFA_ENCRYPTION_KEY must be 32 bytes as 64 hex characters (openssl rand -hex 32)"
            .to_owned()
    };
    let bytes = hex::decode(&raw).map_err(|_| bad())?;
    let bytes: [u8; 32] = bytes.try_into().map_err(|_| bad())?;
    Ok(Some(bytes))
}

/// The key kept in the database; made on first use when `create` is set.
async fn database_key(pool: &PgPool, create: bool) -> Result<Option<[u8; 32]>, CoreError> {
    async fn read(pool: &PgPool) -> Result<Option<[u8; 32]>, CoreError> {
        let stored: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT key FROM penfii.mfa_instance_key WHERE id = 1")
                .fetch_optional(pool)
                .await?;
        stored
            .map(|bytes| {
                bytes
                    .try_into()
                    .map_err(|_| CoreError::Internal("mfa instance key has a wrong length".into()))
            })
            .transpose()
    }
    if let Some(key) = read(pool).await? {
        return Ok(Some(key));
    }
    if !create {
        return Ok(None);
    }
    let mut fresh = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut fresh);
    sqlx::query(
        "INSERT INTO penfii.mfa_instance_key (id, key) VALUES (1, $1) ON CONFLICT (id) DO NOTHING",
    )
    .bind(fresh.as_slice())
    .execute(pool)
    .await?;
    // Whoever inserted first won; everyone uses that key.
    read(pool).await
}

fn configured_key() -> Option<[u8; 32]> {
    env_key(&crate::config::process_env).ok().flatten()
}

async fn sealing_key(pool: &PgPool) -> Result<[u8; 32], CoreError> {
    if let Some(key) = configured_key() {
        return Ok(key);
    }
    database_key(pool, true)
        .await?
        .ok_or_else(|| CoreError::Internal("mfa instance key is missing".into()))
}

/// Opens a stored secret with whichever key sealed it. An unreadable secret (the key it names is not
/// available) is an operator problem and is logged, never a reason to let the sign-in through.
async fn open_secret(
    pool: &PgPool,
    user_id: Uuid,
    stored_key_id: &[u8],
    nonce: &[u8],
    sealed: &[u8],
) -> Result<Option<Vec<u8>>, CoreError> {
    let mut candidates = Vec::new();
    candidates.extend(configured_key());
    candidates.extend(database_key(pool, false).await?);
    for key in candidates {
        if key_id(&key) == stored_key_id {
            return Ok(open(&key, user_id, nonce, sealed));
        }
    }
    tracing::error!(
        %user_id,
        "the key that sealed this account's MFA secret is not available (PENFII_MFA_ENCRYPTION_KEY changed or the database key was lost); recover the account with admin-cli user-mfa-reset"
    );
    Ok(None)
}

// ---- Recovery codes ----------------------------------------------------------------------------

fn new_recovery_code() -> String {
    let mut raw = [0u8; RECOVERY_CODE_CHARS];
    rand::rngs::OsRng.fill_bytes(&mut raw);
    let chars: String = raw
        .iter()
        .map(|byte| RECOVERY_ALPHABET[(byte & 31) as usize] as char)
        .collect();
    chars
        .as_bytes()
        .chunks(4)
        .map(|chunk| std::str::from_utf8(chunk).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("-")
}

/// What a person typed, without separators or case: `None` when it cannot be a code at all.
fn normalize_code(input: &str) -> Option<String> {
    let cleaned: String = input
        .chars()
        .filter(|ch| !ch.is_whitespace() && *ch != '-')
        .flat_map(char::to_lowercase)
        .collect();
    if cleaned.is_empty()
        || cleaned.len() > 32
        || !cleaned.chars().all(|ch| ch.is_ascii_alphanumeric())
    {
        return None;
    }
    Some(cleaned)
}

fn recovery_hash(user_id: Uuid, normalized: &str) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(b"penfii-mfa-recovery:");
    hasher.update(user_id.as_bytes());
    hasher.update(b":");
    hasher.update(normalized.as_bytes());
    hasher.finalize().to_vec()
}

/// Replaces every recovery code of the account by ten new ones; returns them (shown once).
async fn replace_recovery_codes(
    conn: &mut PgConnection,
    user_id: Uuid,
) -> Result<Vec<String>, CoreError> {
    sqlx::query("DELETE FROM penfii.mfa_recovery_codes WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *conn)
        .await?;
    let mut codes = Vec::with_capacity(RECOVERY_CODE_COUNT);
    for _ in 0..RECOVERY_CODE_COUNT {
        let code = new_recovery_code();
        let normalized = normalize_code(&code).expect("a made code is well formed");
        sqlx::query(
            "INSERT INTO penfii.mfa_recovery_codes (id, user_id, code_hash) VALUES ($1, $2, $3)",
        )
        .bind(Uuid::new_v4())
        .bind(user_id)
        .bind(recovery_hash(user_id, &normalized))
        .execute(&mut *conn)
        .await?;
        codes.push(code);
    }
    Ok(codes)
}

// ---- Checking a code ---------------------------------------------------------------------------

/// Which kind of code a normalized input can be.
enum Code {
    Totp(u32),
    Recovery(String),
}

fn classify(input: &str) -> Option<Code> {
    let normalized = normalize_code(input)?;
    if normalized.len() == DIGITS as usize && normalized.chars().all(|ch| ch.is_ascii_digit()) {
        return normalized.parse().ok().map(Code::Totp);
    }
    (normalized.len() == RECOVERY_CODE_CHARS).then_some(Code::Recovery(normalized))
}

struct StoredSecret {
    key_id: Vec<u8>,
    nonce: Vec<u8>,
    sealed: Vec<u8>,
    last_step: i64,
}

async fn stored_secret(
    pool: &PgPool,
    user_id: Uuid,
    pending: bool,
) -> Result<Option<StoredSecret>, CoreError> {
    let row = sqlx::query(
        "SELECT key_id, nonce, secret_ciphertext, last_step FROM penfii.mfa_totp
         WHERE user_id = $1 AND (enabled_at IS NULL) = $2",
    )
    .bind(user_id)
    .bind(pending)
    .fetch_optional(pool)
    .await?;
    row.map(|row| {
        Ok(StoredSecret {
            key_id: row.try_get("key_id")?,
            nonce: row.try_get("nonce")?,
            sealed: row.try_get("secret_ciphertext")?,
            last_step: row.try_get("last_step")?,
        })
    })
    .transpose()
    .map_err(|error: sqlx::Error| error.into())
}

/// The step the code proves, when it is a current code of the account's secret, with the nonce of the
/// sealed secret it was checked against (which identifies that secret).
async fn totp_step(
    pool: &PgPool,
    user_id: Uuid,
    code: u32,
    pending: bool,
) -> Result<Option<(i64, Vec<u8>)>, CoreError> {
    let Some(stored) = stored_secret(pool, user_id, pending).await? else {
        return Ok(None);
    };
    let Some(secret) =
        open_secret(pool, user_id, &stored.key_id, &stored.nonce, &stored.sealed).await?
    else {
        return Ok(None);
    };
    Ok(matching_step(
        &secret,
        code,
        step_at(Utc::now().timestamp()),
        stored.last_step,
    )
    .map(|step| (step, stored.nonce)))
}

/// Checks `input` as an authenticator code or an unused recovery code of an *enabled* account and spends
/// it: a TOTP step is accepted once, a recovery code is marked used in the same statement that finds it.
/// The caller has already counted the attempt.
pub(crate) async fn verify_and_spend(
    pool: &PgPool,
    user_id: Uuid,
    input: &str,
) -> Result<bool, CoreError> {
    match classify(input) {
        None => Ok(false),
        Some(Code::Totp(code)) => {
            let Some((step, _)) = totp_step(pool, user_id, code, false).await? else {
                return Ok(false);
            };
            // Two requests with the same code: only the one that moves `last_step` forward is let in.
            let spent = sqlx::query(
                "UPDATE penfii.mfa_totp SET last_step = $2
                 WHERE user_id = $1 AND enabled_at IS NOT NULL AND last_step < $2",
            )
            .bind(user_id)
            .bind(step)
            .execute(pool)
            .await?;
            Ok(spent.rows_affected() == 1)
        }
        Some(Code::Recovery(normalized)) => {
            let spent = sqlx::query(
                "UPDATE penfii.mfa_recovery_codes SET used_at = now()
                 WHERE user_id = $1 AND code_hash = $2 AND used_at IS NULL
                   AND EXISTS (SELECT 1 FROM penfii.mfa_totp
                               WHERE user_id = $1 AND enabled_at IS NOT NULL)",
            )
            .bind(user_id)
            .bind(recovery_hash(user_id, &normalized))
            .execute(pool)
            .await?;
            Ok(spent.rows_affected() == 1)
        }
    }
}

/// Whether sign-in with the password of this account has to be followed by a code.
pub(crate) async fn is_enabled(pool: &PgPool, user_id: Uuid) -> Result<bool, CoreError> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM penfii.mfa_totp WHERE user_id = $1 AND enabled_at IS NOT NULL)",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await?)
}

/// The per-account counter of code attempts made by the signed-in person (setup, turning off, new
/// recovery codes).
fn account_counter(user_id: &Uuid) -> String {
    user_id.to_string()
}

/// A code asked for by an action of the signed-in account: counted first, verified, and the counter
/// cleared by a right one.
async fn check_code_for_action(pool: &PgPool, user_id: Uuid, input: &str) -> Result<(), CoreError> {
    let key = account_counter(&user_id);
    let counters = [("mfa", key.as_str(), throttle::MFA)];
    throttle::admit_or_limited(pool, &counters).await?;
    if !verify_and_spend(pool, user_id, input).await? {
        return Err(CoreError::MfaCodeInvalid);
    }
    throttle::clear(pool, "mfa", &key).await
}

// ---- The account's own actions -----------------------------------------------------------------

pub async fn status(pool: &PgPool, user_id: Uuid) -> Result<MfaStatus, CoreError> {
    let row = sqlx::query("SELECT enabled_at FROM penfii.mfa_totp WHERE user_id = $1")
        .bind(user_id)
        .fetch_optional(pool)
        .await?;
    let Some(row) = row else {
        return Ok(MfaStatus {
            enabled: false,
            enabled_at: None,
            recovery_codes_remaining: 0,
            setup_pending: false,
        });
    };
    let enabled_at: Option<DateTime<Utc>> = row.try_get("enabled_at")?;
    let remaining: i64 = if enabled_at.is_some() {
        sqlx::query_scalar(
            "SELECT count(*) FROM penfii.mfa_recovery_codes WHERE user_id = $1 AND used_at IS NULL",
        )
        .bind(user_id)
        .fetch_one(pool)
        .await?
    } else {
        0
    };
    Ok(MfaStatus {
        enabled: enabled_at.is_some(),
        enabled_at,
        recovery_codes_remaining: remaining,
        setup_pending: enabled_at.is_none(),
    })
}

/// Starts (or restarts) the setup: a new secret that protects nothing until [`confirm_setup`]. Needs a
/// recent authentication.
pub async fn begin_setup(pool: &PgPool, session: &AuthSession) -> Result<MfaSetup, CoreError> {
    require_recent_authentication(pool, session).await?;
    let user_id = session.user.id;
    let mut secret = [0u8; SECRET_BYTES];
    rand::rngs::OsRng.fill_bytes(&mut secret);
    let key = sealing_key(pool).await?;
    let (nonce, sealed) = seal(&key, user_id, &secret)?;
    // A confirmed account is never overwritten here.
    let started = sqlx::query(
        "INSERT INTO penfii.mfa_totp (user_id, key_id, nonce, secret_ciphertext)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (user_id) DO UPDATE
            SET key_id = $2, nonce = $3, secret_ciphertext = $4, last_step = 0, created_at = now()
            WHERE penfii.mfa_totp.enabled_at IS NULL",
    )
    .bind(user_id)
    .bind(key_id(&key))
    .bind(nonce)
    .bind(sealed)
    .execute(pool)
    .await?;
    if started.rows_affected() == 0 {
        return Err(CoreError::conflict(
            "mfa_already_enabled",
            "two-step verification is already on",
        ));
    }
    Ok(MfaSetup {
        secret: grouped(&base32(&secret)),
        otpauth_uri: otpauth_uri(&session.user.email, &secret),
    })
}

/// Turns MFA on after the person proved they hold the new secret by a current code. Returns the first
/// recovery codes. This device stays signed in, the other web sessions end, connected apps stay.
pub async fn confirm_setup(
    pool: &PgPool,
    session: &AuthSession,
    code: &str,
) -> Result<Vec<String>, CoreError> {
    require_recent_authentication(pool, session).await?;
    let user_id = session.user.id;
    let key = account_counter(&user_id);
    let counters = [("mfa", key.as_str(), throttle::MFA)];
    throttle::admit_or_limited(pool, &counters).await?;
    let Some(Code::Totp(code)) = classify(code) else {
        return Err(CoreError::MfaCodeInvalid);
    };
    if stored_secret(pool, user_id, true).await?.is_none() {
        return Err(CoreError::conflict(
            "mfa_setup_not_started",
            "start the setup first",
        ));
    }
    let Some((step, nonce)) = totp_step(pool, user_id, code, true).await? else {
        return Err(CoreError::MfaCodeInvalid);
    };
    let mut tx = pool.begin().await?;
    // Only the secret the code was checked against is turned on: a setup restarted meanwhile (a new
    // secret, a new nonce) is not enabled by a code of the old one.
    let enabled = sqlx::query(
        "UPDATE penfii.mfa_totp SET enabled_at = now(), last_step = $2
         WHERE user_id = $1 AND enabled_at IS NULL AND nonce = $3",
    )
    .bind(user_id)
    .bind(step)
    .bind(nonce)
    .execute(&mut *tx)
    .await?;
    if enabled.rows_affected() != 1 {
        return Err(CoreError::conflict(
            "mfa_setup_not_started",
            "start the setup first",
        ));
    }
    let codes = replace_recovery_codes(&mut tx, user_id).await?;
    session::end_other_sessions(&mut tx, user_id, session.id).await?;
    audit(&mut tx, user_id, "user.mfa_enabled", "account").await?;
    tx.commit().await?;
    throttle::clear(pool, "mfa", &key).await?;
    Ok(codes)
}

/// Turns MFA off. Needs a recent authentication and a current authenticator or recovery code. This
/// device stays signed in, the other web sessions end, connected apps stay.
pub async fn disable(pool: &PgPool, session: &AuthSession, code: &str) -> Result<(), CoreError> {
    require_recent_authentication(pool, session).await?;
    let user_id = session.user.id;
    if !is_enabled(pool, user_id).await? {
        return Err(CoreError::conflict(
            "mfa_not_enabled",
            "two-step verification is not on",
        ));
    }
    check_code_for_action(pool, user_id, code).await?;
    let mut tx = pool.begin().await?;
    remove_rows(&mut tx, user_id).await?;
    session::end_other_sessions(&mut tx, user_id, session.id).await?;
    audit(&mut tx, user_id, "user.mfa_disabled", "account").await?;
    tx.commit().await?;
    Ok(())
}

/// Ten new recovery codes in place of all the old ones. Same requirements as [`disable`].
pub async fn regenerate_recovery_codes(
    pool: &PgPool,
    session: &AuthSession,
    code: &str,
) -> Result<Vec<String>, CoreError> {
    require_recent_authentication(pool, session).await?;
    let user_id = session.user.id;
    if !is_enabled(pool, user_id).await? {
        return Err(CoreError::conflict(
            "mfa_not_enabled",
            "two-step verification is not on",
        ));
    }
    check_code_for_action(pool, user_id, code).await?;
    let mut tx = pool.begin().await?;
    let codes = replace_recovery_codes(&mut tx, user_id).await?;
    audit(&mut tx, user_id, "user.mfa_recovery_regenerated", "account").await?;
    tx.commit().await?;
    Ok(codes)
}

async fn remove_rows(conn: &mut PgConnection, user_id: Uuid) -> Result<(), CoreError> {
    sqlx::query("DELETE FROM penfii.mfa_recovery_codes WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *conn)
        .await?;
    sqlx::query("DELETE FROM penfii.mfa_totp WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// `admin-cli user-mfa-reset`: the operator's recovery for a person who lost both the authenticator and
/// the recovery codes. Removes MFA and ends every web session and every connected app (OAuth/MCP grant)
/// of the account: whoever held them, the person signs in and connects again. Needs no secret of the
/// account, and the operator never sees one.
pub async fn operator_reset(pool: &PgPool, email: &str) -> Result<Uuid, CoreError> {
    let (_, email_key) = super::normalize_email(email)?;
    let mut tx = pool.begin().await?;
    let user_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM penfii.users WHERE email_key = $1 AND kind = 'HUMAN' FOR UPDATE",
    )
    .bind(&email_key)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(CoreError::NotFound)?;
    remove_rows(&mut tx, user_id).await?;
    crate::admin::end_all_access(&mut tx, user_id, "mfa_reset").await?;
    audit(&mut tx, user_id, "user.mfa_reset", "admin-cli").await?;
    tx.commit().await?;
    // A lock the person ran into while they were locked out does not outlive the recovery.
    throttle::clear(pool, "mfa", &account_counter(&user_id)).await?;
    throttle::clear(pool, "mfa_login", &email_key).await?;
    Ok(user_id)
}

/// A user-level entry of the audit log: it names the account and the way, never a code or a secret.
async fn audit(
    conn: &mut PgConnection,
    user_id: Uuid,
    action: &str,
    via: &str,
) -> Result<(), CoreError> {
    sqlx::query(
        "INSERT INTO penfii.audit_entries
            (id, actor_user_id, action_code, outcome_code, target_kind, target_id, data)
         VALUES ($1, $2, $3, 'OK', 'user', $4, jsonb_build_object('via', $5::text))",
    )
    .bind(Uuid::new_v4())
    .bind((via == "account").then_some(user_id))
    .bind(action)
    .bind(user_id)
    .bind(via)
    .execute(conn)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn totp_matches_the_rfc_6238_sha1_vectors() {
        let secret = b"12345678901234567890";
        // RFC 6238 appendix B, SHA-1, eight digits; the last six are the six-digit code.
        for (time, eight) in [
            (59i64, 94_287_082u32),
            (1_111_111_109, 7_081_804),
            (1_234_567_890, 89_005_924),
            (2_000_000_000, 69_279_037),
        ] {
            assert_eq!(hotp(secret, step_at(time) as u64, 8), eight, "t={time}");
            assert_eq!(hotp(secret, step_at(time) as u64, 6), eight % 1_000_000);
        }
    }

    #[test]
    fn a_step_is_accepted_around_now_and_only_once() {
        let secret = b"12345678901234567890";
        let now = 1_000;
        let code = hotp(secret, now as u64, DIGITS);
        assert_eq!(matching_step(secret, code, now, 0), Some(now));
        assert_eq!(matching_step(secret, code, now + 1, 0), Some(now));
        assert_eq!(matching_step(secret, code, now - 1, 0), Some(now));
        assert_eq!(matching_step(secret, code, now + 2, 0), None, "too old");
        assert_eq!(matching_step(secret, code, now, now), None, "already used");
        assert_eq!(matching_step(secret, code, now, now + 1), None);
    }

    #[test]
    fn base32_is_rfc_4648_without_padding() {
        assert_eq!(base32(b""), "");
        assert_eq!(base32(b"f"), "MY");
        assert_eq!(base32(b"fo"), "MZXQ");
        assert_eq!(base32(b"foo"), "MZXW6");
        assert_eq!(base32(b"foob"), "MZXW6YQ");
        assert_eq!(base32(b"fooba"), "MZXW6YTB");
        assert_eq!(base32(b"foobar"), "MZXW6YTBOI");
    }

    #[test]
    fn the_provisioning_uri_carries_the_parameters_apps_read() {
        let uri = otpauth_uri("a+b@example.com", b"foobar");
        assert!(uri.starts_with("otpauth://totp/Penfii%3Aa%2Bb%40example.com?secret=MZXW6YTBOI"));
        assert!(uri.contains("&issuer=Penfii&algorithm=SHA1&digits=6&period=30"));
    }

    #[test]
    fn recovery_codes_are_typed_loosely_and_stored_hashed() {
        let code = new_recovery_code();
        assert_eq!(code.len(), 14);
        let user = Uuid::new_v4();
        let canonical = normalize_code(&code).unwrap();
        assert_eq!(canonical.len(), RECOVERY_CODE_CHARS);
        let typed = format!(" {} ", code.to_uppercase().replace('-', " "));
        assert_eq!(normalize_code(&typed).unwrap(), canonical);
        assert_ne!(
            recovery_hash(user, &canonical),
            recovery_hash(Uuid::new_v4(), &canonical)
        );
        assert!(matches!(classify(&code), Some(Code::Recovery(_))));
        assert!(matches!(classify("123 456"), Some(Code::Totp(123_456))));
        assert!(classify("12345").is_none());
        assert!(classify("12-34-5!").is_none());
    }

    #[test]
    fn a_sealed_secret_opens_only_with_its_key_and_account() {
        let key = [7u8; 32];
        let user = Uuid::new_v4();
        let (nonce, sealed) = seal(&key, user, b"secret").unwrap();
        assert_eq!(open(&key, user, &nonce, &sealed).unwrap(), b"secret");
        assert!(open(&[8u8; 32], user, &nonce, &sealed).is_none());
        assert!(open(&key, Uuid::new_v4(), &nonce, &sealed).is_none());
        assert_ne!(key_id(&key), key_id(&[8u8; 32]));
    }

    #[test]
    fn the_environment_key_is_optional_and_strict() {
        let with = |value: &str| {
            let value = value.to_owned();
            move |name: &str| (name == "PENFII_MFA_ENCRYPTION_KEY").then(|| value.clone())
        };
        assert_eq!(env_key(&with("")), Ok(None));
        assert_eq!(env_key(&|_: &str| None), Ok(None));
        assert_eq!(env_key(&with(&"ab".repeat(32))).unwrap(), Some([0xab; 32]));
        assert!(env_key(&with("abcd")).is_err());
        assert!(env_key(&with("zz")).is_err());
    }
}
