//! Browser sign-in through an external provider: attempts, the four intents (login, invite,
//! link, reauth), identity linking and unlinking.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use rand::RngCore;
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool, Row};
use uuid::Uuid;

use super::provider::{
    AuthProviders, AuthorizeRequest, CompleteRequest, ExternalProfile, Provider,
};
use super::session::{mark_reauthenticated, require_recent_authentication};
use super::{AuthSession, insert_session, normalize_email, token_hash};
use crate::error::CoreError;
use crate::limits::Limits;

const ATTEMPT_TTL_SECONDS: i64 = 600;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    Login,
    Invite,
    Link,
    Reauth,
}

impl Intent {
    fn as_str(self) -> &'static str {
        match self {
            Self::Login => "login",
            Self::Invite => "invite",
            Self::Link => "link",
            Self::Reauth => "reauth",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "login" => Some(Self::Login),
            "invite" => Some(Self::Invite),
            "link" => Some(Self::Link),
            "reauth" => Some(Self::Reauth),
            _ => None,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInfo {
    pub key: String,
    pub display_name: String,
    pub kind: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityInfo {
    pub provider: String,
    pub display_name: String,
    pub email: Option<String>,
    pub linked_at: DateTime<Utc>,
    pub last_login_at: Option<DateTime<Utc>>,
}

/// The providers a login page may offer.
pub fn offered(providers: &AuthProviders) -> Vec<ProviderInfo> {
    if providers.public_url.is_none() {
        return Vec::new();
    }
    providers
        .list()
        .into_iter()
        .map(|provider| ProviderInfo {
            key: provider.key().to_string(),
            display_name: provider.display_name().to_string(),
            kind: provider.kind().to_string(),
        })
        .collect()
}

/// Upserts the configured providers (never secrets) and disables the ones no longer configured.
pub async fn sync_providers(pool: &PgPool, providers: &AuthProviders) -> Result<(), CoreError> {
    let enabled = providers.public_url.is_some();
    let keys: Vec<String> = providers
        .list()
        .iter()
        .map(|provider| provider.key().to_string())
        .collect();
    sqlx::query(
        "UPDATE penfii.auth_providers SET enabled = false, updated_at = now()
         WHERE provider_key IS NOT NULL AND NOT (provider_key = ANY($1))",
    )
    .bind(&keys)
    .execute(pool)
    .await?;
    for provider in providers.list() {
        sqlx::query(
            "INSERT INTO penfii.auth_providers
                (id, provider_kind, issuer, client_id, secret_ref, enabled, config,
                 provider_key, display_name, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, now())
             ON CONFLICT (provider_key) DO UPDATE SET
                provider_kind = $2, issuer = $3, client_id = $4, secret_ref = $5,
                enabled = $6, config = $7, display_name = $9, updated_at = now()",
        )
        .bind(Uuid::new_v4())
        .bind(provider.kind())
        .bind(provider.issuer())
        .bind(provider.client_id())
        .bind(provider.secret_ref())
        .bind(enabled)
        .bind(provider.config())
        .bind(provider.key())
        .bind(provider.display_name())
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// Only a relative path on this instance.
pub fn safe_return_to(value: Option<&str>) -> String {
    match value {
        Some(path)
            if path.starts_with('/')
                && !path.starts_with("//")
                && !path.contains('\\')
                && !path.chars().any(|ch| ch.is_control()) =>
        {
            path.to_string()
        }
        _ => "/".to_string(),
    }
}

fn random_url_token() -> String {
    let mut raw = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut raw);
    URL_SAFE_NO_PAD.encode(raw)
}

pub struct StartInput<'a> {
    pub provider_key: &'a str,
    pub intent: Intent,
    pub session: Option<&'a AuthSession>,
    pub invitation_token: Option<&'a str>,
    pub return_to: Option<&'a str>,
    pub address: Option<&'a str>,
    pub throttle: &'a crate::limits::ThrottleLimits,
}

/// Creates the attempt and returns the provider URL the browser goes to.
pub async fn start(
    pool: &PgPool,
    providers: &AuthProviders,
    input: StartInput<'_>,
) -> Result<String, CoreError> {
    // Every start counts against the address: an attempt row and a provider round trip per call.
    super::check_address(pool, "auth_start", input.address).await?;
    super::record_address_failure(pool, "auth_start", input.address, input.throttle).await?;
    let provider = providers
        .get(input.provider_key)
        .ok_or(CoreError::NotFound)?
        .clone();
    let redirect_uri = providers
        .redirect_uri(input.provider_key)
        .ok_or(CoreError::NotFound)?;
    let provider_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM penfii.auth_providers WHERE provider_key = $1 AND enabled",
    )
    .bind(input.provider_key)
    .fetch_optional(pool)
    .await?
    .ok_or(CoreError::NotFound)?;

    let mut user_id = None;
    let mut session_id = None;
    let mut invitation_id = None;
    match input.intent {
        Intent::Login => {}
        Intent::Invite => {
            let token = input
                .invitation_token
                .ok_or_else(|| CoreError::validation("invitation token is required"))?;
            invitation_id = Some(
                sqlx::query_scalar::<_, Uuid>(
                    "SELECT id FROM penfii.workspace_invitations
                     WHERE token_hash = $1 AND revoked_at IS NULL AND accepted_at IS NULL
                       AND expires_at > now()",
                )
                .bind(token_hash(token))
                .fetch_optional(pool)
                .await?
                .ok_or(CoreError::NotFound)?,
            );
        }
        Intent::Link | Intent::Reauth => {
            let session = input.session.ok_or(CoreError::Unauthenticated)?;
            user_id = Some(session.user.id);
            session_id = Some(session.id);
            if input.intent == Intent::Link {
                require_recent_authentication(pool, session).await?;
            } else {
                let linked: bool = sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM penfii.external_identities
                                    WHERE user_id = $1 AND provider_id = $2)",
                )
                .bind(session.user.id)
                .bind(provider_id)
                .fetch_one(pool)
                .await?;
                if !linked {
                    return Err(CoreError::NotFound);
                }
            }
        }
    }

    let state = random_url_token();
    let verifier = random_url_token();
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let nonce = random_url_token();
    sqlx::query("DELETE FROM penfii.auth_attempts WHERE created_at < now() - interval '1 day'")
        .execute(pool)
        .await?;
    sqlx::query(
        "INSERT INTO penfii.auth_attempts
            (id, state_hash, provider_id, intent, pkce_verifier, nonce, user_id, session_id,
             invitation_id, return_to, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, now() + make_interval(secs => $11))",
    )
    .bind(Uuid::new_v4())
    .bind(token_hash(&state))
    .bind(provider_id)
    .bind(input.intent.as_str())
    .bind(&verifier)
    .bind(&nonce)
    .bind(user_id)
    .bind(session_id)
    .bind(invitation_id)
    .bind(safe_return_to(input.return_to))
    .bind(ATTEMPT_TTL_SECONDS as f64)
    .execute(pool)
    .await?;

    provider
        .authorize_url(AuthorizeRequest {
            redirect_uri: &redirect_uri,
            state: &state,
            code_challenge: &challenge,
            nonce: &nonce,
            force_login: input.intent == Intent::Reauth,
            want_email: input.intent == Intent::Invite,
        })
        .await
}

pub struct CallbackInput<'a> {
    pub provider_key: &'a str,
    pub code: Option<&'a str>,
    pub state: Option<&'a str>,
    /// The provider's `error` parameter, if it sent one.
    pub error: Option<&'a str>,
    /// The browser's current session cookie, if any.
    pub session_token: Option<&'a str>,
    pub device: Option<&'a str>,
}

#[derive(Debug)]
pub struct CallbackOutcome {
    /// Relative path to send the browser to; never carries a code, token or e-mail.
    pub redirect: String,
    /// A new session cookie (token, ttl seconds).
    pub new_session: Option<(String, i64)>,
    /// The presented session was revoked and its cookie should be cleared.
    pub clear_session: bool,
}

struct Attempt {
    provider_id: Uuid,
    intent: Intent,
    verifier: String,
    nonce: String,
    user_id: Option<Uuid>,
    session_id: Option<Uuid>,
    invitation_id: Option<Uuid>,
    return_to: String,
}

fn with_query(path: &str, key: &str, value: &str) -> String {
    let separator = if path.contains('?') { '&' } else { '?' };
    let pair = url::form_urlencoded::Serializer::new(String::new())
        .append_pair(key, value)
        .finish();
    format!("{path}{separator}{pair}")
}

fn error_code(error: &CoreError) -> &'static str {
    match error {
        CoreError::Conflict { code, .. } | CoreError::Rule { code, .. } => match *code {
            "identity_linked_elsewhere" => "identity_linked_elsewhere",
            "provider_already_linked" => "provider_already_linked",
            "not_linked" => "not_linked",
            "wrong_account" => "wrong_account",
            "email_mismatch" => "email_mismatch",
            "account_exists_sign_in" => "account_exists_sign_in",
            "id_token_invalid" => "id_token_invalid",
            "invalid_attempt" => "invalid_attempt",
            _ => "provider_error",
        },
        CoreError::ReauthRequired { .. } => "reauth_required",
        CoreError::Unauthenticated => "unauthenticated",
        CoreError::NotFound => "invalid_attempt",
        _ => "provider_error",
    }
}

/// Why a browser sign-in ended in `?auth_error=`: the provider, the code and a fixed reason, never the
/// user's identity. Without this a blocked egress or a wrong client secret is invisible to the operator.
fn log_failure(provider_key: &str, error: &CoreError) {
    let reason = match error {
        CoreError::Conflict { message, .. } => message.as_str(),
        _ => "",
    };
    tracing::warn!(
        provider = provider_key,
        code = error_code(error),
        reason,
        "external sign-in failed"
    );
}

pub async fn callback(
    pool: &PgPool,
    limits: &Limits,
    providers: &AuthProviders,
    input: CallbackInput<'_>,
) -> CallbackOutcome {
    let failed = |return_to: &str, code: &str| CallbackOutcome {
        redirect: with_query(return_to, "auth_error", code),
        new_session: None,
        clear_session: false,
    };
    let (Some(provider), Some(redirect_uri), Some(state)) = (
        providers.get(input.provider_key),
        providers.redirect_uri(input.provider_key),
        input.state,
    ) else {
        return failed("/", "invalid_attempt");
    };

    // Single use even when the rest fails: the attempt is consumed first.
    let consumed = sqlx::query(
        "UPDATE penfii.auth_attempts a SET consumed_at = now()
         FROM penfii.auth_providers p
         WHERE a.state_hash = $1 AND a.consumed_at IS NULL AND a.expires_at > now()
           AND p.id = a.provider_id AND p.provider_key = $2
         RETURNING a.provider_id, a.intent, a.pkce_verifier, a.nonce, a.user_id, a.session_id,
                   a.invitation_id, a.return_to",
    )
    .bind(token_hash(state))
    .bind(input.provider_key)
    .fetch_optional(pool)
    .await;
    let attempt = match consumed {
        Ok(Some(row)) => {
            let intent: String = row.try_get("intent").unwrap_or_default();
            let Some(intent) = Intent::parse(&intent) else {
                return failed("/", "invalid_attempt");
            };
            Attempt {
                provider_id: row.try_get("provider_id").unwrap_or_default(),
                intent,
                verifier: row.try_get("pkce_verifier").unwrap_or_default(),
                nonce: row.try_get("nonce").unwrap_or_default(),
                user_id: row.try_get("user_id").unwrap_or_default(),
                session_id: row.try_get("session_id").unwrap_or_default(),
                invitation_id: row.try_get("invitation_id").unwrap_or_default(),
                return_to: row.try_get("return_to").unwrap_or_else(|_| "/".into()),
            }
        }
        _ => return failed("/", "invalid_attempt"),
    };
    let back = safe_return_to(Some(&attempt.return_to));
    if input.error.is_some() {
        return failed(&back, "provider_denied");
    }
    let Some(code) = input.code else {
        return failed(&back, "invalid_attempt");
    };
    let profile = match provider
        .complete(CompleteRequest {
            redirect_uri: &redirect_uri,
            code,
            code_verifier: &attempt.verifier,
            nonce: &attempt.nonce,
            want_email: attempt.intent == Intent::Invite,
        })
        .await
    {
        Ok(profile) => profile,
        Err(error) => {
            log_failure(input.provider_key, &error);
            return failed(&back, error_code(&error));
        }
    };

    match act(
        pool,
        limits,
        provider.as_ref(),
        &attempt,
        profile,
        input.session_token,
        input.device,
    )
    .await
    {
        Ok(mut outcome) => {
            outcome.redirect = if outcome.redirect.is_empty() {
                back
            } else {
                outcome.redirect
            };
            outcome
        }
        Err(error) => {
            if let CoreError::Internal(message) = &error {
                tracing::error!("{message}");
            }
            log_failure(input.provider_key, &error);
            failed(&back, error_code(&error))
        }
    }
}

async fn act(
    pool: &PgPool,
    limits: &Limits,
    provider: &dyn Provider,
    attempt: &Attempt,
    profile: ExternalProfile,
    presented: Option<&str>,
    device: Option<&str>,
) -> Result<CallbackOutcome, CoreError> {
    let back = safe_return_to(Some(&attempt.return_to));
    match attempt.intent {
        Intent::Login => {
            let ttl = session_ttl(limits)?;
            let mut tx = pool.begin().await?;
            let user_id: Option<Uuid> = sqlx::query_scalar(
                "SELECT i.user_id FROM penfii.external_identities i
                 JOIN penfii.users u ON u.id = i.user_id
                 WHERE i.issuer = $1 AND i.subject = $2 AND i.provider_id = $3
                   AND u.disabled_at IS NULL",
            )
            .bind(&profile.issuer)
            .bind(&profile.subject)
            .bind(attempt.provider_id)
            .fetch_optional(&mut *tx)
            .await?;
            let user_id = match user_id {
                Some(user_id) => {
                    refresh_identity(&mut tx, &profile, attempt.provider_id).await?;
                    user_id
                }
                None => sign_up(&mut tx, provider, &profile, attempt.provider_id).await?,
            };
            let token = insert_session(&mut tx, user_id, ttl, provider.key(), device).await?;
            tx.commit().await?;
            revoke_presented(pool, presented).await?;
            Ok(CallbackOutcome {
                redirect: back,
                new_session: Some((token, ttl)),
                clear_session: false,
            })
        }
        Intent::Invite => {
            let ttl = session_ttl(limits)?;
            let invitation_id = attempt.invitation_id.ok_or(CoreError::NotFound)?;
            let mut tx = pool.begin().await?;
            let invite = sqlx::query(
                "SELECT workspace_id, email_key, role, team_id, team_name FROM penfii.workspace_invitations
                 WHERE id = $1 AND revoked_at IS NULL AND accepted_at IS NULL
                   AND expires_at > now()
                 FOR UPDATE",
            )
            .bind(invitation_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(CoreError::NotFound)?;
            let invite_email: String = invite.try_get("email_key")?;
            let workspace_id: Uuid = invite.try_get("workspace_id")?;
            let role: String = invite.try_get("role")?;
            let invited_team: Option<Uuid> = invite.try_get("team_id")?;
            let invited_team_name: Option<String> = invite.try_get("team_name")?;
            let matches = profile.email_proves_ownership
                && profile
                    .email
                    .as_deref()
                    .and_then(|email| normalize_email(email).ok())
                    .is_some_and(|(_, key)| key == invite_email);
            if !matches {
                return Err(CoreError::conflict(
                    "email_mismatch",
                    "the provider does not vouch for the invited e-mail",
                ));
            }
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM penfii.users WHERE email_key = $1)",
            )
            .bind(&invite_email)
            .fetch_one(&mut *tx)
            .await?;
            if exists {
                return Err(CoreError::conflict(
                    "account_exists_sign_in",
                    "an account with this e-mail exists; sign in and accept the invitation",
                ));
            }
            let display_name = profile
                .display_name
                .clone()
                .filter(|name| super::require_label(name, "displayName").is_ok())
                .unwrap_or_else(|| invite_email.split('@').next().unwrap_or("user").to_string());
            let user_id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO penfii.users (id, email, email_key, email_verified_at, display_name)
                 VALUES ($1, $2, $2, now(), $3)",
            )
            .bind(user_id)
            .bind(&invite_email)
            .bind(&display_name)
            .execute(&mut *tx)
            .await?;
            insert_identity(&mut tx, user_id, &profile, attempt.provider_id).await?;
            sqlx::query(
                "INSERT INTO penfii.workspace_members (workspace_id, user_id, role, state)
                 VALUES ($1, $2, $3, 'ACTIVE') ON CONFLICT (workspace_id, user_id) DO NOTHING",
            )
            .bind(workspace_id)
            .bind(user_id)
            .bind(&role)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE penfii.workspace_invitations SET accepted_at = now(), accepted_user_id = $2
                 WHERE id = $1",
            )
            .bind(invitation_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
            let team = crate::organization::place_invited_team(
                &mut tx,
                workspace_id,
                user_id,
                invitation_id,
                invited_team,
                invited_team_name.as_deref(),
            )
            .await?;
            let token = insert_session(&mut tx, user_id, ttl, provider.key(), device).await?;
            tx.commit().await?;
            revoke_presented(pool, presented).await?;
            // The Team step's outcome travels with the redirect so the page can say it.
            let redirect = match team {
                Some(outcome) => {
                    let with_status = with_query(&back, "teamOutcome", &outcome.status);
                    with_query(&with_status, "teamName", &outcome.team_name)
                }
                None => back,
            };
            Ok(CallbackOutcome {
                redirect,
                new_session: Some((token, ttl)),
                clear_session: false,
            })
        }
        Intent::Link | Intent::Reauth => {
            let token = presented.ok_or(CoreError::Unauthenticated)?;
            let session = super::authenticate_session(pool, token).await?;
            if Some(session.id) != attempt.session_id || Some(session.user.id) != attempt.user_id {
                return Err(CoreError::Unauthenticated);
            }
            if attempt.intent == Intent::Link {
                require_recent_authentication(pool, &session).await?;
                let mut tx = pool.begin().await?;
                insert_identity(&mut tx, session.user.id, &profile, attempt.provider_id).await?;
                tx.commit().await?;
                Ok(CallbackOutcome {
                    redirect: with_query(&back, "auth", "linked"),
                    new_session: None,
                    clear_session: false,
                })
            } else {
                // The provider must return the person's own identity, not just some login.
                let mut tx = pool.begin().await?;
                let own: Option<(String, String)> = sqlx::query_as(
                    "SELECT issuer, subject FROM penfii.external_identities
                     WHERE user_id = $1 AND provider_id = $2",
                )
                .bind(session.user.id)
                .bind(attempt.provider_id)
                .fetch_optional(&mut *tx)
                .await?;
                if own != Some((profile.issuer.clone(), profile.subject.clone())) {
                    return Err(CoreError::conflict(
                        "wrong_account",
                        "the provider returned a different account",
                    ));
                }
                refresh_identity(&mut tx, &profile, attempt.provider_id).await?;
                tx.commit().await?;
                mark_reauthenticated(pool, session.id, &format!("provider:{}", provider.key()))
                    .await?;
                Ok(CallbackOutcome {
                    redirect: with_query(&back, "auth", "reauthenticated"),
                    new_session: None,
                    clear_session: false,
                })
            }
        }
    }
}

/// First sign-in with an identity nobody linked. Only when the operator turned sign-up on for this provider
/// and the provider vouches for the e-mail: a new user is created and this identity is linked to it in the
/// caller's transaction. An existing account with the same e-mail is never linked, never hinted at and never
/// duplicated: the answer is the same `not_linked` as when sign-up is off.
async fn sign_up(
    tx: &mut PgConnection,
    provider: &dyn Provider,
    profile: &ExternalProfile,
    provider_id: Uuid,
) -> Result<Uuid, CoreError> {
    let not_linked = |reason: &str| CoreError::conflict("not_linked", reason.to_string());
    if !provider.signup_allowed() {
        return Err(not_linked("no account is linked to this identity"));
    }
    let email = profile
        .email
        .as_deref()
        .filter(|_| profile.email_proves_ownership)
        .ok_or_else(|| not_linked("sign-up needs an e-mail the provider vouches for"))?;
    let (_, email_key) =
        normalize_email(email).map_err(|_| not_linked("the provider e-mail is invalid"))?;
    let display_name = profile
        .display_name
        .clone()
        .filter(|name| super::require_label(name, "displayName").is_ok())
        .unwrap_or_else(|| email_key.split('@').next().unwrap_or("user").to_string());
    let user_id = Uuid::new_v4();
    // The unique e-mail key decides a race between two first sign-ins and an existing account alike.
    let created = sqlx::query(
        "INSERT INTO penfii.users (id, email, email_key, email_verified_at, display_name)
         VALUES ($1, $2, $2, now(), $3) ON CONFLICT (email_key) DO NOTHING",
    )
    .bind(user_id)
    .bind(&email_key)
    .bind(&display_name)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if created == 0 {
        return Err(not_linked(
            "an account with this e-mail exists and is not linked to this identity",
        ));
    }
    insert_identity(tx, user_id, profile, provider_id).await?;
    Ok(user_id)
}

fn session_ttl(limits: &Limits) -> Result<i64, CoreError> {
    limits.session_ttl_seconds.ok_or_else(|| {
        CoreError::conflict("session_ttl_unconfigured", "session ttl is not configured")
    })
}

/// A sign-in never keeps or upgrades the session cookie that came with it.
async fn revoke_presented(pool: &PgPool, presented: Option<&str>) -> Result<(), CoreError> {
    if let Some(token) = presented.filter(|token| !token.is_empty()) {
        super::logout(pool, token).await?;
    }
    Ok(())
}

async fn insert_identity(
    conn: &mut PgConnection,
    user_id: Uuid,
    profile: &ExternalProfile,
    provider_id: Uuid,
) -> Result<(), CoreError> {
    let inserted = sqlx::query(
        "INSERT INTO penfii.external_identities
            (issuer, subject, user_id, provider_id, email, email_verified, display_name,
             avatar_url, claims, last_login_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, now())",
    )
    .bind(&profile.issuer)
    .bind(&profile.subject)
    .bind(user_id)
    .bind(provider_id)
    .bind(&profile.email)
    .bind(profile.email_verified)
    .bind(&profile.display_name)
    .bind(&profile.avatar_url)
    .bind(&profile.claims)
    .execute(&mut *conn)
    .await;
    match inserted {
        Ok(_) => Ok(()),
        Err(sqlx::Error::Database(db)) if db.is_unique_violation() => {
            // Which unique index tells the two cases apart.
            if db
                .constraint()
                .is_some_and(|name| name.contains("user_provider"))
            {
                Err(CoreError::conflict(
                    "provider_already_linked",
                    "this account already has an identity at this provider",
                ))
            } else {
                Err(CoreError::conflict(
                    "identity_linked_elsewhere",
                    "this identity belongs to another account",
                ))
            }
        }
        Err(error) => Err(error.into()),
    }
}

/// Provider details refresh at each sign-in; `users.email` is never touched.
async fn refresh_identity(
    conn: &mut PgConnection,
    profile: &ExternalProfile,
    provider_id: Uuid,
) -> Result<(), CoreError> {
    sqlx::query(
        "UPDATE penfii.external_identities
         SET email = $4, email_verified = $5, display_name = $6, avatar_url = $7, claims = $8,
             last_login_at = now(), updated_at = now()
         WHERE issuer = $1 AND subject = $2 AND provider_id = $3",
    )
    .bind(&profile.issuer)
    .bind(&profile.subject)
    .bind(provider_id)
    .bind(&profile.email)
    .bind(profile.email_verified)
    .bind(&profile.display_name)
    .bind(&profile.avatar_url)
    .bind(&profile.claims)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

pub async fn list_identities(pool: &PgPool, user_id: Uuid) -> Result<Vec<IdentityInfo>, CoreError> {
    let rows = sqlx::query(
        "SELECT p.provider_key, COALESCE(p.display_name, p.provider_key) AS display_name,
                i.email, i.linked_at, i.last_login_at
         FROM penfii.external_identities i
         JOIN penfii.auth_providers p ON p.id = i.provider_id
         WHERE i.user_id = $1 ORDER BY i.linked_at",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|row| {
            Ok(IdentityInfo {
                provider: row.try_get("provider_key")?,
                display_name: row.try_get("display_name")?,
                email: row.try_get("email")?,
                linked_at: row.try_get("linked_at")?,
                last_login_at: row.try_get("last_login_at")?,
            })
        })
        .collect::<Result<_, sqlx::Error>>()
        .map_err(Into::into)
}

/// Locks the user, counts the methods and refuses to leave none.
async fn remove_method(
    pool: &PgPool,
    session: &AuthSession,
    provider_key: Option<&str>,
) -> Result<(), CoreError> {
    require_recent_authentication(pool, session).await?;
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT id FROM penfii.users WHERE id = $1 FOR UPDATE")
        .bind(session.user.id)
        .fetch_one(&mut *tx)
        .await?;
    let methods: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM penfii.password_credentials WHERE user_id = $1)
              + (SELECT count(*) FROM penfii.external_identities WHERE user_id = $1)",
    )
    .bind(session.user.id)
    .fetch_one(&mut *tx)
    .await?;
    let removed = match provider_key {
        Some(key) => sqlx::query(
            "DELETE FROM penfii.external_identities i USING penfii.auth_providers p
                 WHERE i.user_id = $1 AND p.id = i.provider_id AND p.provider_key = $2",
        )
        .bind(session.user.id)
        .bind(key)
        .execute(&mut *tx)
        .await?
        .rows_affected(),
        None => sqlx::query("DELETE FROM penfii.password_credentials WHERE user_id = $1")
            .bind(session.user.id)
            .execute(&mut *tx)
            .await?
            .rows_affected(),
    };
    if removed == 0 {
        return Err(CoreError::NotFound);
    }
    if methods <= 1 {
        return Err(CoreError::conflict(
            "last_method",
            "this is the only way to sign in to the account",
        ));
    }
    tx.commit().await?;
    Ok(())
}

pub async fn unlink_identity(
    pool: &PgPool,
    session: &AuthSession,
    provider_key: &str,
) -> Result<(), CoreError> {
    remove_method(pool, session, Some(provider_key)).await
}

pub async fn remove_password(pool: &PgPool, session: &AuthSession) -> Result<(), CoreError> {
    remove_method(pool, session, None).await
}

#[derive(Debug)]
pub struct AdminIdentities {
    pub user_id: Uuid,
    pub disabled: bool,
    pub has_password: bool,
    pub identities: Vec<IdentityInfo>,
}

/// `admin-cli user-identities`: what a locked-out account can still sign in with.
pub async fn admin_identities(pool: &PgPool, email: &str) -> Result<AdminIdentities, CoreError> {
    let (_, email_key) = normalize_email(email)?;
    let row = sqlx::query(
        "SELECT u.id, u.disabled_at IS NOT NULL AS disabled,
                EXISTS (SELECT 1 FROM penfii.password_credentials p WHERE p.user_id = u.id) AS has_password
         FROM penfii.users u WHERE u.email_key = $1",
    )
    .bind(&email_key)
    .fetch_optional(pool)
    .await?
    .ok_or(CoreError::NotFound)?;
    let user_id: Uuid = row.try_get("id")?;
    Ok(AdminIdentities {
        user_id,
        disabled: row.try_get("disabled")?,
        has_password: row.try_get("has_password")?,
        identities: list_identities(pool, user_id).await?,
    })
}

/// `admin-cli user-set-password`: sets or replaces the password, ends every session of the
/// account and writes an audit entry.
pub async fn admin_set_password(
    pool: &PgPool,
    email: &str,
    password: &str,
) -> Result<Uuid, CoreError> {
    let (email, email_key) = normalize_email(email)?;
    super::validate_new_password(password, &email)?;
    // Hashed before the transaction (and its row lock) opens.
    let password_hash = super::hash_password(password).await?;
    let mut tx = pool.begin().await?;
    let user_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM penfii.users WHERE email_key = $1 AND kind = 'HUMAN' FOR UPDATE",
    )
    .bind(&email_key)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(CoreError::NotFound)?;
    sqlx::query(
        "INSERT INTO penfii.password_credentials (user_id, password_hash) VALUES ($1, $2)
         ON CONFLICT (user_id) DO UPDATE SET password_hash = $2, changed_at = now()",
    )
    .bind(user_id)
    .bind(&password_hash)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE penfii.sessions SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL",
    )
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO penfii.audit_entries
            (id, actor_user_id, action_code, outcome_code, target_kind, target_id, data)
         VALUES ($1, NULL, 'user.password_set', 'OK', 'user', $2, '{\"via\":\"admin-cli\"}')",
    )
    .bind(Uuid::new_v4())
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(user_id)
}
