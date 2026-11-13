//! Users, passwords, and sessions. No OAuth routes in this slice.

use std::sync::OnceLock;

use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use chrono::{DateTime, Utc};
use penfii_contracts::User;
use rand::RngCore;
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool, Row};
use uuid::Uuid;

use crate::error::CoreError;
use crate::files::ObjectStore;
use crate::limits::Limits;

mod external;
mod password;
pub mod provider;
mod reset;
mod session;
mod throttle;

pub use external::{
    AdminIdentities, CallbackInput, CallbackOutcome, IdentityInfo, Intent, ProviderInfo,
    StartInput, admin_identities, admin_set_password, callback, list_identities, offered,
    remove_password, safe_return_to, start, sync_providers, unlink_identity,
};

pub use password::{
    MAX_CHARS as PASSWORD_MAX_CHARS, MIN_CHARS as PASSWORD_MIN_CHARS, validate_new_password,
};
pub use reset::{
    admit_request as admit_reset_request, confirm as confirm_password_reset,
    process_request as process_reset_request,
};

pub use session::{
    AuthSession, SessionInfo, authenticate_session, check_address, list_sessions, reauth_methods,
    reauth_window_seconds, reauth_with_password, record_address_failure,
    require_recent_authentication, revoke_other_sessions, revoke_session,
};
pub(crate) use session::{reauth_required, recently_authenticated};

#[derive(Debug)]
pub struct LoginResult {
    pub user: User,
    pub token: String,
    pub ttl_seconds: i64,
}

#[derive(Debug)]
pub struct BootstrapInput {
    pub email: String,
    pub password: String,
    pub display_name: String,
    pub workspace_name: String,
    pub time_zone: String,
}

#[derive(Debug)]
pub struct BootstrapOutput {
    pub user_id: Uuid,
    pub workspace_id: Uuid,
}

struct LoginRow {
    id: Uuid,
    email: String,
    display_name: String,
    locale: Option<String>,
    time_zone: Option<String>,
    email_verified_at: Option<DateTime<Utc>>,
    disabled_at: Option<DateTime<Utc>>,
    password_hash: Option<String>,
}

pub fn normalize_email(email: &str) -> Result<(String, String), CoreError> {
    let trimmed = email.trim();
    if trimmed.is_empty()
        || trimmed.len() > 320
        || !trimmed.contains('@')
        || trimmed
            .chars()
            .any(|ch| ch.is_control() || ch.is_whitespace())
    {
        return Err(CoreError::validation("email is invalid"));
    }
    Ok((trimmed.to_string(), trimmed.to_lowercase()))
}

pub fn require_label(value: &str, field: &str) -> Result<String, CoreError> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.chars().count() > 200
        || trimmed.chars().any(|ch| ch.is_control())
    {
        return Err(CoreError::validation(format!("{field} is invalid")));
    }
    Ok(trimmed.to_string())
}

/// The shape of a password that is being *checked* (sign-in, re-authentication). A password that
/// is being *set* goes through [`validate_new_password`] instead.
pub fn require_password(password: &str) -> Result<(), CoreError> {
    if password.is_empty() || password.len() > 1024 {
        return Err(CoreError::validation("password is invalid"));
    }
    Ok(())
}

pub async fn login(
    pool: &PgPool,
    limits: &Limits,
    email: &str,
    password: &str,
) -> Result<LoginResult, CoreError> {
    login_from(pool, limits, email, password, None, None).await
}

/// `address` feeds the per-address throttle; `device` is a coarse label for the session list.
pub async fn login_from(
    pool: &PgPool,
    limits: &Limits,
    email: &str,
    password: &str,
    address: Option<&str>,
    device: Option<&str>,
) -> Result<LoginResult, CoreError> {
    let ttl = limits.session_ttl_seconds.ok_or_else(|| {
        CoreError::conflict("session_ttl_unconfigured", "session ttl is not configured")
    })?;
    let (_, email_key) = normalize_email(email)?;
    require_password(password)?;
    // Three counters, see `ThrottleLimits`. Whoever is full refuses the attempt before the password
    // is looked at, so a locked key answers the same for a right and a wrong password.
    let pair_key = account_address_key(&email_key, address);
    check_address(pool, "login", address).await?;
    throttle::check(pool, "login", &email_key).await?;
    throttle::check(pool, "login_account_ip", &pair_key).await?;

    // The row is read on the pool and no connection is held while the password is hashed: the
    // transaction opens only for the session insert (see the transaction rules in docs/development.md).
    let row = sqlx::query_as::<_, LoginRow>(
        "SELECT u.id, u.email, u.display_name, u.locale, u.time_zone, u.email_verified_at,
                u.disabled_at, p.password_hash
         FROM penfii.users u
         LEFT JOIN penfii.password_credentials p ON p.user_id = u.id
         WHERE u.email_key = $1 AND u.kind = 'HUMAN'",
    )
    .bind(&email_key)
    .fetch_optional(pool)
    .await?;

    let valid = match &row {
        Some(user) if user.disabled_at.is_none() => match user.password_hash.as_deref() {
            Some(hash) => verify_password(password, hash).await,
            None => false,
        },
        _ => {
            verify_dummy(password).await;
            false
        }
    };
    if !valid {
        let throttle_limits = &limits.login_throttle;
        throttle::record_failure_limit(pool, "login", &email_key, throttle_limits.account).await?;
        throttle::record_failure_limit(
            pool,
            "login_account_ip",
            &pair_key,
            throttle_limits.account_address,
        )
        .await?;
        record_address_failure(pool, "login", address, throttle_limits).await?;
        return Err(CoreError::Unauthenticated);
    }
    let user = row.expect("valid login has a user");
    let mut tx = pool.begin().await?;
    let token = insert_session(&mut tx, user.id, ttl, "password", device).await?;
    let summary = user_from_login(&user);
    tx.commit().await?;
    throttle::clear(pool, "login", &email_key).await?;
    throttle::clear(pool, "login_account_ip", &pair_key).await?;
    Ok(LoginResult {
        user: summary,
        token,
        ttl_seconds: ttl,
    })
}

/// The key of "this e-mail from this address". An unknown address (tests, direct calls) is one
/// shared address, so the small per-account-and-address limit still applies there.
fn account_address_key(email_key: &str, address: Option<&str>) -> String {
    format!("{email_key}\n{}", address.unwrap_or("-"))
}

pub async fn authenticate(pool: &PgPool, token: &str) -> Result<User, CoreError> {
    let hash = token_hash(token);
    let row = sqlx::query_as::<_, UserRow>(
        "SELECT u.id, u.email, u.display_name, u.locale, u.time_zone, u.email_verified_at,
                u.job_title, u.status_emoji, u.status_text, u.status_clear_at,
                (u.avatar_object_key IS NOT NULL) AS has_avatar
         FROM penfii.sessions s
         JOIN penfii.users u ON u.id = s.user_id
         WHERE s.token_hash = $1
           AND s.revoked_at IS NULL
           AND s.expires_at > now()
           AND u.disabled_at IS NULL",
    )
    .bind(hash)
    .fetch_optional(pool)
    .await?;
    row.map(user_from_row).ok_or(CoreError::Unauthenticated)
}

pub async fn logout(pool: &PgPool, token: &str) -> Result<(), CoreError> {
    let hash = token_hash(token);
    sqlx::query(
        "UPDATE penfii.sessions SET revoked_at = now()
         WHERE token_hash = $1 AND revoked_at IS NULL",
    )
    .bind(hash)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn bootstrap(pool: &PgPool, input: BootstrapInput) -> Result<BootstrapOutput, CoreError> {
    let (email, email_key) = normalize_email(&input.email)?;
    validate_new_password(&input.password, &email)?;
    let display_name = require_label(&input.display_name, "displayName")?;
    let workspace_name = require_label(&input.workspace_name, "workspaceName")?;
    let time_zone = require_label(&input.time_zone, "timeZone")?;
    // Hashed before the transaction opens: no connection is held while Argon2 runs.
    let password_hash = hash_password(&input.password).await?;

    let mut tx = pool.begin().await?;
    sqlx::query("LOCK TABLE penfii.users IN EXCLUSIVE MODE")
        .execute(&mut *tx)
        .await?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM penfii.users")
        .fetch_one(&mut *tx)
        .await?;
    if count > 0 {
        return Err(CoreError::conflict(
            "already_bootstrapped",
            "a user already exists",
        ));
    }
    let user_id = insert_user(
        &mut tx,
        &email,
        &email_key,
        &display_name,
        &password_hash,
        true,
    )
    .await?;
    let workspace_id =
        insert_owned_workspace(&mut tx, user_id, &workspace_name, &time_zone).await?;
    tx.commit().await?;
    Ok(BootstrapOutput {
        user_id,
        workspace_id,
    })
}

pub async fn create_account(
    pool: &PgPool,
    input: BootstrapInput,
) -> Result<BootstrapOutput, CoreError> {
    let (email, email_key) = normalize_email(&input.email)?;
    validate_new_password(&input.password, &email)?;
    let display_name = require_label(&input.display_name, "displayName")?;
    let workspace_name = require_label(&input.workspace_name, "workspaceName")?;
    let time_zone = require_label(&input.time_zone, "timeZone")?;
    let password_hash = hash_password(&input.password).await?;

    let mut tx = pool.begin().await?;
    let user_id = insert_user(
        &mut tx,
        &email,
        &email_key,
        &display_name,
        &password_hash,
        true,
    )
    .await?;
    let workspace_id =
        insert_owned_workspace(&mut tx, user_id, &workspace_name, &time_zone).await?;
    tx.commit().await?;
    Ok(BootstrapOutput {
        user_id,
        workspace_id,
    })
}

pub struct ProfileUpdate {
    pub display_name: String,
    pub email: String,
    /// Compatibility: a request that still carries the current password is an inline password
    /// reauthentication. Without it the session must be recently authenticated.
    pub current_password: Option<String>,
    pub new_password: Option<String>,
}

pub async fn update_profile(
    pool: &PgPool,
    session: &AuthSession,
    update: ProfileUpdate,
) -> Result<User, CoreError> {
    let user_id = session.user.id;
    let (email, email_key) = normalize_email(&update.email)?;
    let display_name = require_label(&update.display_name, "displayName")?;
    let next_password = match update.new_password.as_deref() {
        Some(password) if !password.is_empty() => {
            validate_new_password(password, &email)?;
            Some(password)
        }
        _ => None,
    };
    match update.current_password.as_deref() {
        Some(password) if !password.is_empty() => {
            reauth_with_password(pool, session, password).await?;
        }
        _ => require_recent_authentication(pool, session).await?,
    }
    let next_hash = match next_password {
        Some(password) => Some(hash_password(password).await?),
        None => None,
    };

    let mut tx = pool.begin().await?;
    let updated = sqlx::query(
        "UPDATE penfii.users SET email = $2, email_key = $3, display_name = $4 WHERE id = $1",
    )
    .bind(user_id)
    .bind(&email)
    .bind(&email_key)
    .bind(&display_name)
    .execute(&mut *tx)
    .await;
    if let Err(error) = updated {
        if is_unique(&error) {
            return Err(CoreError::conflict(
                "account_exists",
                "account already exists",
            ));
        }
        return Err(error.into());
    }
    if let Some(password_hash) = next_hash {
        // Setting a password where none exists is part of the same reauthenticated action.
        sqlx::query(
            "INSERT INTO penfii.password_credentials (user_id, password_hash)
             VALUES ($1, $2)
             ON CONFLICT (user_id) DO UPDATE SET password_hash = $2, changed_at = now()",
        )
        .bind(user_id)
        .bind(password_hash)
        .execute(&mut *tx)
        .await?;
    }
    let row = sqlx::query_as::<_, UserRow>(&format!(
        "SELECT {USER_COLUMNS} FROM penfii.users WHERE id = $1"
    ))
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(user_from_row(row))
}

pub(crate) async fn insert_user(
    conn: &mut PgConnection,
    email: &str,
    email_key: &str,
    display_name: &str,
    password_hash: &str,
    verified: bool,
) -> Result<Uuid, CoreError> {
    let user_id = Uuid::new_v4();
    let inserted = sqlx::query(
        "INSERT INTO penfii.users
            (id, email, email_key, email_verified_at, display_name)
         VALUES ($1, $2, $3, CASE WHEN $4 THEN now() END, $5)",
    )
    .bind(user_id)
    .bind(email)
    .bind(email_key)
    .bind(verified)
    .bind(display_name)
    .execute(&mut *conn)
    .await;
    if let Err(error) = inserted {
        if is_unique(&error) {
            return Err(CoreError::conflict(
                "account_exists",
                "account already exists",
            ));
        }
        return Err(error.into());
    }
    sqlx::query(
        "INSERT INTO penfii.password_credentials (user_id, password_hash)
         VALUES ($1, $2)",
    )
    .bind(user_id)
    .bind(password_hash)
    .execute(&mut *conn)
    .await?;
    Ok(user_id)
}

pub(crate) async fn insert_owned_workspace(
    conn: &mut PgConnection,
    user_id: Uuid,
    name: &str,
    time_zone: &str,
) -> Result<Uuid, CoreError> {
    let workspace_id = Uuid::new_v4();
    sqlx::query("INSERT INTO penfii.workspaces (id, name, time_zone) VALUES ($1, $2, $3)")
        .bind(workspace_id)
        .bind(name)
        .bind(time_zone)
        .execute(&mut *conn)
        .await?;
    sqlx::query(
        "INSERT INTO penfii.workspace_members (workspace_id, user_id, role, state)
         VALUES ($1, $2, 'OWNER', 'ACTIVE')",
    )
    .bind(workspace_id)
    .bind(user_id)
    .execute(&mut *conn)
    .await?;
    sqlx::query("INSERT INTO penfii.workspace_auth_state (workspace_id) VALUES ($1)")
        .bind(workspace_id)
        .execute(&mut *conn)
        .await?;
    sqlx::query("INSERT INTO penfii.workspace_graph_state (workspace_id) VALUES ($1)")
        .bind(workspace_id)
        .execute(&mut *conn)
        .await?;
    Ok(workspace_id)
}

// Argon2 takes tens of milliseconds of CPU on purpose, so the async wrappers run it on the blocking
// pool instead of stalling a runtime worker (and every request scheduled on it).

pub(crate) async fn hash_password(password: &str) -> Result<String, CoreError> {
    let password = password.to_owned();
    tokio::task::spawn_blocking(move || argon2_hash(&password))
        .await
        .map_err(|error| CoreError::Internal(error.to_string()))?
}

pub(crate) async fn verify_password(password: &str, encoded: &str) -> bool {
    let (password, encoded) = (password.to_owned(), encoded.to_owned());
    tokio::task::spawn_blocking(move || argon2_verify(&password, &encoded))
        .await
        .unwrap_or(false)
}

/// Spends the same work as a real check when there is no account, so the answer time does not tell.
async fn verify_dummy(password: &str) {
    let password = password.to_owned();
    let _ = tokio::task::spawn_blocking(move || argon2_verify(&password, dummy_hash())).await;
}

fn argon2_hash(password: &str) -> Result<String, CoreError> {
    let salt = SaltString::generate(&mut rand::rngs::OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| CoreError::Internal(error.to_string()))
}

fn argon2_verify(password: &str, encoded: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(encoded) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

pub(crate) fn new_secret() -> (String, Vec<u8>) {
    let mut raw = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut raw);
    let token = hex::encode(raw);
    let hash = token_hash(&token);
    (token, hash)
}

pub(crate) fn token_hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

pub(crate) async fn insert_session(
    conn: &mut PgConnection,
    user_id: Uuid,
    ttl_seconds: i64,
    method: &str,
    device: Option<&str>,
) -> Result<String, CoreError> {
    let (token, hash) = new_secret();
    sqlx::query(
        "INSERT INTO penfii.sessions
            (id, user_id, token_hash, expires_at,
             authenticated_at, last_reauth_at, last_reauth_method, auth_method, last_seen_at,
             device_label)
         VALUES ($1, $2, $3, now() + make_interval(secs => $4), now(), now(), $5, $5, now(), $6)",
    )
    .bind(Uuid::new_v4())
    .bind(user_id)
    .bind(hash)
    .bind(ttl_seconds as f64)
    .bind(method)
    .bind(device)
    .execute(&mut *conn)
    .await?;
    Ok(token)
}

const USER_COLUMNS: &str = "id, email, display_name, locale, time_zone, email_verified_at, job_title, status_emoji, status_text, status_clear_at, (avatar_object_key IS NOT NULL) AS has_avatar";

pub(crate) struct UserRow {
    id: Uuid,
    email: String,
    display_name: String,
    locale: Option<String>,
    time_zone: Option<String>,
    email_verified_at: Option<DateTime<Utc>>,
    job_title: Option<String>,
    status_emoji: Option<String>,
    status_text: Option<String>,
    status_clear_at: Option<DateTime<Utc>>,
    has_avatar: bool,
}

/// A user value with no profile extras. Login answers before the profile columns are read.
pub(crate) fn plain_user(
    id: Uuid,
    email: String,
    display_name: String,
    locale: Option<String>,
    time_zone: Option<String>,
    email_verified_at: Option<DateTime<Utc>>,
) -> User {
    User {
        id,
        email,
        display_name,
        locale,
        time_zone,
        email_verified_at,
        job_title: None,
        status_emoji: None,
        status_text: None,
        status_clear_at: None,
        has_avatar: false,
    }
}

fn user_from_login(row: &LoginRow) -> User {
    plain_user(
        row.id,
        row.email.clone(),
        row.display_name.clone(),
        row.locale.clone(),
        row.time_zone.clone(),
        row.email_verified_at,
    )
}

pub(crate) fn user_from_row(row: UserRow) -> User {
    let (status_emoji, status_text, status_clear_at) =
        shown_status(row.status_emoji, row.status_text, row.status_clear_at);
    User {
        id: row.id,
        email: row.email,
        display_name: row.display_name,
        locale: row.locale,
        time_zone: row.time_zone,
        email_verified_at: row.email_verified_at,
        job_title: row.job_title.filter(|value| !value.is_empty()),
        status_emoji,
        status_text,
        status_clear_at,
        has_avatar: row.has_avatar,
    }
}

/// A status whose clear time has passed is shown as none. The row is left as stored.
pub(crate) fn shown_status(
    emoji: Option<String>,
    text: Option<String>,
    clear_at: Option<DateTime<Utc>>,
) -> (Option<String>, Option<String>, Option<DateTime<Utc>>) {
    if clear_at.is_some_and(|at| at <= Utc::now()) {
        return (None, None, None);
    }
    let emoji = emoji.filter(|value| !value.is_empty());
    let text = text.filter(|value| !value.is_empty());
    if emoji.is_none() && text.is_none() {
        return (None, None, None);
    }
    (emoji, text, clear_at)
}

/// Profile fields that are not credentials. Changing them does not ask for a fresh sign-in.
pub struct PublicProfile {
    pub display_name: String,
    pub job_title: Option<String>,
    pub time_zone: Option<String>,
    pub status_emoji: Option<String>,
    pub status_text: Option<String>,
    pub status_clear_at: Option<DateTime<Utc>>,
}

pub async fn update_public_profile(
    pool: &PgPool,
    user_id: Uuid,
    update: PublicProfile,
) -> Result<User, CoreError> {
    let display_name = require_label(&update.display_name, "displayName")?;
    let job_title = optional_text(update.job_title.as_deref(), "jobTitle", 80)?;
    let time_zone = optional_zone(update.time_zone.as_deref())?;
    let mut status_emoji = optional_text(update.status_emoji.as_deref(), "statusEmoji", 16)?;
    let mut status_text = optional_text(update.status_text.as_deref(), "statusText", 80)?;
    let mut status_clear_at = update.status_clear_at;
    if status_emoji.is_none() && status_text.is_none() {
        status_clear_at = None;
    }
    // An expired clear time is stored as no status, so the row matches what the next read shows.
    if status_clear_at.is_some_and(|at| at <= Utc::now()) {
        status_emoji = None;
        status_text = None;
        status_clear_at = None;
    }
    sqlx::query(
        "UPDATE penfii.users
         SET display_name = $2, job_title = $3, time_zone = $4,
             status_emoji = $5, status_text = $6, status_clear_at = $7
         WHERE id = $1",
    )
    .bind(user_id)
    .bind(&display_name)
    .bind(&job_title)
    .bind(&time_zone)
    .bind(&status_emoji)
    .bind(&status_text)
    .bind(status_clear_at)
    .execute(pool)
    .await?;
    load_user(pool, user_id).await
}

pub async fn load_user(pool: &PgPool, user_id: Uuid) -> Result<User, CoreError> {
    let row = sqlx::query_as::<_, UserRow>(&format!(
        "SELECT {USER_COLUMNS} FROM penfii.users WHERE id = $1"
    ))
    .bind(user_id)
    .fetch_optional(pool)
    .await?
    .ok_or(CoreError::NotFound)?;
    Ok(user_from_row(row))
}

const AVATAR_MAX_BYTES: usize = 512 * 1024;

/// Where one generation of a profile photo lives. A new photo uses the next revision, so a
/// replacement never overwrites the object a reader might still be fetching.
pub fn avatar_object_key(user_id: Uuid, revision: i64) -> String {
    format!("users/{user_id}/avatars/{revision}")
}

pub async fn set_avatar(
    pool: &PgPool,
    store: &ObjectStore,
    user_id: Uuid,
    bytes: bytes::Bytes,
    declared_mime: &str,
) -> Result<(), CoreError> {
    if bytes.is_empty() || bytes.len() > AVATAR_MAX_BYTES {
        return Err(CoreError::rule(
            "avatar_too_large",
            format!(
                "avatar must be an image of at most {} KB",
                AVATAR_MAX_BYTES / 1024
            ),
        ));
    }
    let mime = sniff_image(&bytes)?;
    let declared = declared_mime
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if !declared.is_empty()
        && declared != "application/octet-stream"
        && declared != mime
        && declared.starts_with("image/")
    {
        return Err(CoreError::rule(
            "avatar_not_image",
            "the file contents do not match its type",
        ));
    }
    // Two attempts cover one concurrent replacement. A lost race deletes the object it just wrote.
    for _ in 0..2 {
        let current = sqlx::query(
            "SELECT avatar_object_key, avatar_revision FROM penfii.users WHERE id = $1",
        )
        .bind(user_id)
        .fetch_optional(pool)
        .await?
        .ok_or(CoreError::NotFound)?;
        let previous: Option<String> = current.try_get("avatar_object_key")?;
        let revision: i64 = current.try_get("avatar_revision")?;
        let next = revision
            .checked_add(1)
            .ok_or_else(|| CoreError::Internal("avatar revision".into()))?;
        let key = avatar_object_key(user_id, next);
        store
            .put(&key, bytes.clone())
            .await
            .map_err(CoreError::Internal)?;
        let updated = sqlx::query(
            "UPDATE penfii.users
             SET avatar_object_key = $2, avatar_mime = $3, avatar_revision = $4
             WHERE id = $1 AND avatar_revision = $5",
        )
        .bind(user_id)
        .bind(&key)
        .bind(mime)
        .bind(next)
        .bind(revision)
        .execute(pool)
        .await?;
        if updated.rows_affected() == 1 {
            if let Some(old) = previous.filter(|old| old != &key) {
                // The row no longer names this object. A failed delete leaves an unreferenced
                // generation; the next photo uses a new key, so it cannot be served by mistake.
                let _ = store.delete(&old).await;
            }
            return Ok(());
        }
        let _ = store.delete(&key).await;
    }
    Err(CoreError::conflict(
        "avatar_conflict",
        "the photo was updated at the same time",
    ))
}

pub async fn clear_avatar(
    pool: &PgPool,
    store: &ObjectStore,
    user_id: Uuid,
) -> Result<(), CoreError> {
    let mut tx = pool.begin().await?;
    let current = sqlx::query(
        "SELECT avatar_object_key, avatar_revision FROM penfii.users WHERE id = $1 FOR UPDATE",
    )
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(CoreError::NotFound)?;
    let previous: Option<String> = current.try_get("avatar_object_key")?;
    let revision: i64 = current.try_get("avatar_revision")?;
    let next = revision
        .checked_add(1)
        .ok_or_else(|| CoreError::Internal("avatar revision".into()))?;
    sqlx::query(
        "UPDATE penfii.users
         SET avatar_object_key = NULL, avatar_mime = NULL, avatar_revision = $2
         WHERE id = $1",
    )
    .bind(user_id)
    .bind(next)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    if let Some(old) = previous {
        store.delete(&old).await.map_err(CoreError::Internal)?;
    }
    Ok(())
}

/// The photo, if the two people share an active workspace, or the caller is asking for their own.
pub async fn read_avatar(
    pool: &PgPool,
    store: &ObjectStore,
    actor: Uuid,
    target: Uuid,
) -> Result<(Vec<u8>, String), CoreError> {
    if actor != target {
        let shares: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1 FROM penfii.workspace_members a
                JOIN penfii.workspace_members b ON b.workspace_id = a.workspace_id
                WHERE a.user_id = $1 AND b.user_id = $2
                  AND a.state = 'ACTIVE' AND b.state = 'ACTIVE'
             )",
        )
        .bind(actor)
        .bind(target)
        .fetch_one(pool)
        .await?;
        if !shares {
            return Err(CoreError::NotFound);
        }
    }
    let row = sqlx::query("SELECT avatar_object_key, avatar_mime FROM penfii.users WHERE id = $1")
        .bind(target)
        .fetch_optional(pool)
        .await?
        .ok_or(CoreError::NotFound)?;
    let key: Option<String> = row.try_get("avatar_object_key")?;
    let mime: Option<String> = row.try_get("avatar_mime")?;
    let (Some(key), Some(mime)) = (key, mime) else {
        return Err(CoreError::NotFound);
    };
    match store.get(&key).await {
        Ok(bytes) if !bytes.is_empty() => Ok((bytes, mime)),
        Ok(_) => Err(CoreError::NotFound),
        Err(error) if error.to_ascii_lowercase().contains("not found") => Err(CoreError::NotFound),
        Err(error) => Err(CoreError::Internal(error)),
    }
}

fn optional_text(
    value: Option<&str>,
    field: &str,
    max_chars: usize,
) -> Result<Option<String>, CoreError> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if value.chars().count() > max_chars || value.chars().any(|ch| ch.is_control()) {
        return Err(CoreError::validation(format!("{field} is invalid")));
    }
    Ok(Some(value.to_string()))
}

fn optional_zone(value: Option<&str>) -> Result<Option<String>, CoreError> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let ok = value.chars().count() <= 64
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '_' | '+' | '-'));
    if !ok {
        return Err(CoreError::validation("timeZone is invalid"));
    }
    Ok(Some(value.to_string()))
}

pub(crate) fn sniff_image(bytes: &[u8]) -> Result<&'static str, CoreError> {
    let mime = if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        "image/png"
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        "image/gif"
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else {
        return Err(CoreError::rule("avatar_not_image", "choose an image file"));
    };
    Ok(mime)
}

fn dummy_hash() -> &'static str {
    static DUMMY: OnceLock<String> = OnceLock::new();
    DUMMY.get_or_init(|| argon2_hash("penfii-dummy-password").unwrap_or_default())
}

fn is_unique(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::Database(db) if db.is_unique_violation())
}

impl sqlx::FromRow<'_, sqlx::postgres::PgRow> for LoginRow {
    fn from_row(row: &sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;
        Ok(Self {
            id: row.try_get("id")?,
            email: row.try_get("email")?,
            display_name: row.try_get("display_name")?,
            locale: row.try_get("locale")?,
            time_zone: row.try_get("time_zone")?,
            email_verified_at: row.try_get("email_verified_at")?,
            disabled_at: row.try_get("disabled_at")?,
            password_hash: row.try_get("password_hash")?,
        })
    }
}

impl sqlx::FromRow<'_, sqlx::postgres::PgRow> for UserRow {
    fn from_row(row: &sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;
        Ok(Self {
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
        })
    }
}

#[cfg(test)]
mod hash_tests {
    use super::*;

    /// OWASP Password Storage Cheat Sheet: Argon2id with at least 19 MiB of memory, 2 iterations and
    /// 1 degree of parallelism (or an equivalent trade-off). The library defaults are checked here so a
    /// dependency update that weakens them is noticed.
    #[test]
    fn stored_passwords_use_argon2id_at_or_above_the_owasp_minimum() {
        let stored = argon2_hash("a long enough sample password").unwrap();
        let parsed = PasswordHash::new(&stored).unwrap();
        assert_eq!(parsed.algorithm.as_str(), "argon2id");
        let params = argon2::Params::try_from(&parsed).unwrap();
        assert!(
            params.m_cost() >= 19 * 1024,
            "memory {} KiB",
            params.m_cost()
        );
        assert!(params.t_cost() >= 2, "iterations {}", params.t_cost());
        assert!(params.p_cost() >= 1);
        assert_ne!(
            stored,
            argon2_hash("a long enough sample password").unwrap(),
            "salted"
        );
    }

    #[test]
    fn avatar_bytes_are_accepted_only_when_they_are_a_known_image() {
        assert_eq!(
            sniff_image(&[0xFF, 0xD8, 0xFF, 0x00]).unwrap(),
            "image/jpeg"
        );
        assert_eq!(sniff_image(b"\x89PNG\r\n\x1a\nrest").unwrap(), "image/png");
        assert!(sniff_image(b"not an image").is_err());
        assert!(sniff_image(&[]).is_err());
    }
}
