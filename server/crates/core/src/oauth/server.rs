//! Authorization endpoint logic, consent, token endpoint (grant-type registry), revocation,
//! bearer authentication and grants.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Duration, Utc};
use rand::RngCore;
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool, Row};
use url::Url;
use uuid::Uuid;

use super::client::{self, Client};
use super::{OAuthConfig, policy, scopes};
use crate::error::CoreError;
use crate::identity::{AuthSession, require_recent_authentication};

const REQUEST_TTL_SECONDS: i64 = 600;
const CODE_TTL_SECONDS: i64 = 60;

/// An RFC 6749 §5.2 error: `{"error": …, "error_description": …}` with an HTTP status.
#[derive(Debug)]
pub struct OAuthError {
    pub status: u16,
    pub error: &'static str,
    pub description: String,
}

impl OAuthError {
    fn new(status: u16, error: &'static str, description: impl Into<String>) -> Self {
        Self {
            status,
            error,
            description: description.into(),
        }
    }

    fn invalid_grant(description: &str) -> Self {
        Self::new(400, "invalid_grant", description)
    }

    fn invalid_request(description: &str) -> Self {
        Self::new(400, "invalid_request", description)
    }

    pub fn body(&self) -> Value {
        json!({"error": self.error, "error_description": self.description})
    }
}

impl From<sqlx::Error> for OAuthError {
    fn from(error: sqlx::Error) -> Self {
        tracing::error!(%error, "oauth database error");
        Self::new(500, "server_error", "internal error")
    }
}

impl From<CoreError> for OAuthError {
    fn from(error: CoreError) -> Self {
        match error {
            CoreError::Internal(message) => {
                tracing::error!(%message, "oauth internal error");
                Self::new(500, "server_error", "internal error")
            }
            CoreError::NotFound => Self::new(400, "invalid_client", "unknown client"),
            other => Self::new(400, "invalid_request", format!("{other:?}")),
        }
    }
}

pub fn random_token(prefix: &str) -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!("{prefix}{}", URL_SAFE_NO_PAD.encode(bytes))
}

fn digest(value: &str) -> Vec<u8> {
    Sha256::digest(value.as_bytes()).to_vec()
}

// ---------------------------------------------------------------------------------------------
// Authorization endpoint
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Default, Clone)]
pub struct AuthorizeParams {
    pub response_type: Option<String>,
    pub client_id: Option<String>,
    pub redirect_uri: Option<String>,
    pub state: Option<String>,
    pub code_challenge: Option<String>,
    pub code_challenge_method: Option<String>,
    pub scope: Option<String>,
    pub resource: Option<String>,
}

#[derive(Debug)]
pub enum AuthorizeResult {
    /// Continue in the SPA at `/consent?request=<id>`.
    Consent(Uuid),
    /// The client or redirect URI cannot be trusted: show a Penfii error page, never redirect.
    Page(String),
    /// Redirect back to the client with an error (and `iss`).
    Redirect(String),
}

pub fn redirect_with(
    config: &OAuthConfig,
    redirect_uri: &str,
    state: Option<&str>,
    pairs: &[(&str, &str)],
) -> String {
    let mut url = Url::parse(redirect_uri).expect("validated redirect uri");
    {
        let mut query = url.query_pairs_mut();
        for (key, value) in pairs {
            query.append_pair(key, value);
        }
        if let Some(state) = state {
            query.append_pair("state", state);
        }
        query.append_pair("iss", config.issuer());
    }
    url.to_string()
}

/// Validates the whole request server-side and stores it.
pub async fn begin_authorization(
    pool: &PgPool,
    config: &OAuthConfig,
    params: AuthorizeParams,
) -> Result<AuthorizeResult, CoreError> {
    let Some(client_id) = params.client_id.as_deref().filter(|id| !id.is_empty()) else {
        return Ok(AuthorizeResult::Page(
            "The request has no client_id.".into(),
        ));
    };
    let client = match client::resolve(pool, config, client_id).await {
        Ok(client) => client,
        Err(CoreError::Internal(message)) => return Err(CoreError::Internal(message)),
        Err(_) => {
            return Ok(AuthorizeResult::Page(
                "This application could not be identified.".into(),
            ));
        }
    };
    let redirect_uri = match params.redirect_uri.as_deref() {
        Some(uri) if client::redirect_matches(&client.redirect_uris, uri) => uri.to_string(),
        None if client.redirect_uris.len() == 1 => client.redirect_uris[0].clone(),
        _ => {
            return Ok(AuthorizeResult::Page(
                "The redirect address does not match the application.".into(),
            ));
        }
    };
    let state = params.state.as_deref();
    let fail = |error: &str, description: &str| {
        AuthorizeResult::Redirect(redirect_with(
            config,
            &redirect_uri,
            state,
            &[("error", error), ("error_description", description)],
        ))
    };
    if params.response_type.as_deref() != Some("code") {
        return Ok(fail("unsupported_response_type", "only code is supported"));
    }
    let challenge = params.code_challenge.as_deref().unwrap_or("");
    if params.code_challenge_method.as_deref() != Some("S256")
        || challenge.len() < 43
        || challenge.len() > 128
        || !challenge
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~'))
    {
        return Ok(fail("invalid_request", "PKCE with S256 is required"));
    }
    if let Some(resource) = params.resource.as_deref()
        && resource != config.resource()
    {
        return Ok(fail("invalid_target", "unknown resource"));
    }
    let mut requested = scopes::normalize(params.scope.as_deref().unwrap_or(""));
    if requested.is_empty() {
        requested = scopes::default_request()
            .into_iter()
            .map(str::to_string)
            .collect();
    }
    // Housekeeping: finished requests and codes older than a day.
    let _ = sqlx::query(
        "DELETE FROM penfii.oauth_authorization_requests WHERE expires_at < now() - interval '1 day'",
    )
    .execute(pool)
    .await;
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO penfii.oauth_authorization_requests
            (id, client_pk, redirect_uri, state, code_challenge, scope_requested, resource, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, now() + make_interval(secs => $8))",
    )
    .bind(id)
    .bind(client.pk)
    .bind(&redirect_uri)
    .bind(params.state.as_deref())
    .bind(challenge)
    .bind(&requested)
    .bind(config.resource())
    .bind(REQUEST_TTL_SECONDS as f64)
    .execute(pool)
    .await?;
    Ok(AuthorizeResult::Consent(id))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsentScope {
    pub scope: String,
    pub high_risk: bool,
    /// Changes data (not `*:read`): shown unchecked until the person ticks it.
    pub change: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsentView {
    pub id: Uuid,
    pub client_name: String,
    pub client_id: String,
    pub redirect_host: String,
    pub loopback: bool,
    pub scopes: Vec<ConsentScope>,
    /// Set when the person already has a grant for this client; approving replaces it.
    pub replaces_existing: bool,
    pub expires_at: DateTime<Utc>,
}

struct RequestRow {
    client: Client,
    redirect_uri: String,
    state: Option<String>,
    code_challenge: String,
    scopes: Vec<String>,
    resource: String,
    expires_at: DateTime<Utc>,
}

/// Loads a pending request for the signed-in user. The first person to load it owns it;
/// anyone else, an expired, consumed or unknown request is `NotFound`.
async fn load_request(
    conn: &mut PgConnection,
    user_id: Uuid,
    request_id: Uuid,
    for_update: bool,
) -> Result<RequestRow, CoreError> {
    let sql = format!(
        "SELECT r.client_pk, r.user_id, r.redirect_uri, r.state, r.code_challenge,
                r.scope_requested, r.resource, r.expires_at,
                c.client_id, c.kind, c.client_name, c.redirect_uris
         FROM penfii.oauth_authorization_requests r
         JOIN penfii.oauth_clients c ON c.id = r.client_pk
         WHERE r.id = $1 AND r.consumed_at IS NULL AND r.expires_at > now() {}",
        if for_update { "FOR UPDATE OF r" } else { "" }
    );
    let row = sqlx::query(&sql)
        .bind(request_id)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or(CoreError::NotFound)?;
    let owner: Option<Uuid> = row.try_get("user_id")?;
    match owner {
        Some(owner) if owner != user_id => return Err(CoreError::NotFound),
        Some(_) => {}
        None => {
            sqlx::query(
                "UPDATE penfii.oauth_authorization_requests SET user_id = $2
                 WHERE id = $1 AND user_id IS NULL",
            )
            .bind(request_id)
            .bind(user_id)
            .execute(&mut *conn)
            .await?;
        }
    }
    let uris: Value = row.try_get("redirect_uris")?;
    Ok(RequestRow {
        client: Client {
            pk: row.try_get("client_pk")?,
            client_id: row.try_get("client_id")?,
            kind: row.try_get("kind")?,
            name: row.try_get("client_name")?,
            redirect_uris: uris
                .as_array()
                .map(|all| {
                    all.iter()
                        .filter_map(|uri| uri.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
        },
        redirect_uri: row.try_get("redirect_uri")?,
        state: row.try_get("state")?,
        code_challenge: row.try_get("code_challenge")?,
        scopes: row.try_get("scope_requested")?,
        resource: row.try_get("resource")?,
        expires_at: row.try_get("expires_at")?,
    })
}

pub async fn consent_view(
    pool: &PgPool,
    user_id: Uuid,
    request_id: Uuid,
) -> Result<ConsentView, CoreError> {
    let mut conn = pool.acquire().await?;
    let request = load_request(&mut conn, user_id, request_id, false).await?;
    let existing: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM penfii.oauth_grants
                        WHERE user_id = $1 AND client_pk = $2 AND revoked_at IS NULL)",
    )
    .bind(user_id)
    .bind(request.client.pk)
    .fetch_one(&mut *conn)
    .await?;
    let host = Url::parse(&request.redirect_uri)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .unwrap_or_default();
    Ok(ConsentView {
        id: request_id,
        client_name: request.client.name.clone(),
        client_id: request.client.client_id.clone(),
        redirect_host: host,
        loopback: client::is_loopback_redirect(&request.redirect_uri),
        scopes: request
            .scopes
            .iter()
            .map(|scope| ConsentScope {
                scope: scope.clone(),
                high_risk: scopes::is_high_risk(scope),
                change: scopes::is_change(scope),
            })
            .collect(),
        replaces_existing: existing,
        expires_at: request.expires_at,
    })
}

#[derive(Debug)]
pub enum Decision {
    Approve { scopes: Vec<String> },
    Deny,
}

/// The decision is the only state-changing step. Returns the URL to send the person
/// back to the client with. Consuming the request is atomic.
pub async fn decide(
    pool: &PgPool,
    config: &OAuthConfig,
    session: &AuthSession,
    request_id: Uuid,
    decision: Decision,
) -> Result<String, CoreError> {
    let user_id = session.user.id;
    let mut tx = pool.begin().await?;
    let request = load_request(&mut tx, user_id, request_id, true).await?;
    let consumed = sqlx::query(
        "UPDATE penfii.oauth_authorization_requests SET consumed_at = now()
         WHERE id = $1 AND consumed_at IS NULL",
    )
    .bind(request_id)
    .execute(&mut *tx)
    .await?;
    if consumed.rows_affected() != 1 {
        return Err(CoreError::NotFound);
    }
    let state = request.state.as_deref();
    let chosen = match decision {
        Decision::Deny => {
            tx.commit().await?;
            return Ok(redirect_with(
                config,
                &request.redirect_uri,
                state,
                &[
                    ("error", "access_denied"),
                    ("error_description", "the user denied the request"),
                ],
            ));
        }
        Decision::Approve { scopes } => scopes,
    };
    // The person can remove scopes, never add.
    let granted: Vec<String> = request
        .scopes
        .iter()
        .filter(|scope| chosen.contains(scope))
        .cloned()
        .collect();
    if chosen.iter().any(|scope| !request.scopes.contains(scope)) {
        return Err(CoreError::validation("scope was not requested"));
    }
    if granted.is_empty() {
        return Err(CoreError::validation("select at least one permission"));
    }
    // Destructive and administrative capability needs recent authentication at consent.
    if granted.iter().any(|scope| scopes::is_high_risk(scope)) {
        require_recent_authentication(pool, session).await?;
    }
    // One live grant per (user, client): a new consent replaces it and kills its tokens.
    sqlx::query(
        "UPDATE penfii.oauth_tokens SET revoked_at = now()
         WHERE revoked_at IS NULL AND grant_id IN
            (SELECT id FROM penfii.oauth_grants
             WHERE user_id = $1 AND client_pk = $2 AND revoked_at IS NULL)",
    )
    .bind(user_id)
    .bind(request.client.pk)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE penfii.oauth_grants SET revoked_at = now(), revoked_reason = 'replaced'
         WHERE user_id = $1 AND client_pk = $2 AND revoked_at IS NULL",
    )
    .bind(user_id)
    .bind(request.client.pk)
    .execute(&mut *tx)
    .await?;
    let grant_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO penfii.oauth_grants (id, user_id, client_pk, scope) VALUES ($1, $2, $3, $4)",
    )
    .bind(grant_id)
    .bind(user_id)
    .bind(request.client.pk)
    .bind(&granted)
    .execute(&mut *tx)
    .await?;
    let code = random_token("pfc_");
    sqlx::query(
        "INSERT INTO penfii.oauth_codes
            (code_hash, grant_id, client_pk, redirect_uri, code_challenge, resource, scope, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, now() + make_interval(secs => $8))",
    )
    .bind(digest(&code))
    .bind(grant_id)
    .bind(request.client.pk)
    .bind(&request.redirect_uri)
    .bind(&request.code_challenge)
    .bind(&request.resource)
    .bind(&granted)
    .bind(CODE_TTL_SECONDS as f64)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    policy::audit(
        pool,
        policy::AuditRow {
            user_id,
            workspace_id: None,
            action_code: "oauth.grant_created",
            outcome_code: "OK",
            target_kind: "oauth_grant",
            target_id: Some(grant_id),
            data: json!({"clientPk": request.client.pk, "clientName": request.client.name, "scopes": granted}),
        },
    )
    .await;
    Ok(redirect_with(
        config,
        &request.redirect_uri,
        state,
        &[("code", code.as_str())],
    ))
}

// ---------------------------------------------------------------------------------------------
// Token endpoint
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Default, Clone)]
pub struct TokenRequest {
    pub grant_type: Option<String>,
    pub code: Option<String>,
    pub redirect_uri: Option<String>,
    pub client_id: Option<String>,
    pub code_verifier: Option<String>,
    pub refresh_token: Option<String>,
    pub scope: Option<String>,
    pub resource: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: &'static str,
    pub expires_in: i64,
    pub refresh_token: String,
    pub scope: String,
}

/// The grant-type registry: an unknown `grant_type` is `unsupported_grant_type`; another grant
/// (for example enterprise-managed authorization) is one more arm.
pub async fn token(
    pool: &PgPool,
    config: &OAuthConfig,
    request: TokenRequest,
) -> Result<TokenResponse, OAuthError> {
    match request.grant_type.as_deref() {
        Some("authorization_code") => authorization_code_grant(pool, config, request).await,
        Some("refresh_token") => refresh_grant(pool, config, request).await,
        _ => Err(OAuthError::new(
            400,
            "unsupported_grant_type",
            "grant_type must be authorization_code or refresh_token",
        )),
    }
}

fn pkce_matches(verifier: &str, challenge: &str) -> bool {
    if verifier.len() < 43 || verifier.len() > 128 {
        return false;
    }
    let computed = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    // Constant-time comparison.
    computed.len() == challenge.len()
        && computed
            .bytes()
            .zip(challenge.bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

async fn authorization_code_grant(
    pool: &PgPool,
    config: &OAuthConfig,
    request: TokenRequest,
) -> Result<TokenResponse, OAuthError> {
    let (Some(code), Some(verifier), Some(client_id), Some(redirect_uri)) = (
        request.code.as_deref(),
        request.code_verifier.as_deref(),
        request.client_id.as_deref(),
        request.redirect_uri.as_deref(),
    ) else {
        return Err(OAuthError::invalid_request(
            "code, code_verifier, client_id and redirect_uri are required",
        ));
    };
    let mut tx = pool.begin().await?;
    let row = sqlx::query(
        "SELECT k.grant_id, k.client_pk, k.redirect_uri, k.code_challenge, k.resource, k.scope,
                k.expires_at, k.used_at, k.family_id, c.client_id
         FROM penfii.oauth_codes k JOIN penfii.oauth_clients c ON c.id = k.client_pk
         WHERE k.code_hash = $1 FOR UPDATE OF k",
    )
    .bind(digest(code))
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| OAuthError::invalid_grant("the code is not valid"))?;
    let used_at: Option<DateTime<Utc>> = row.try_get("used_at")?;
    let grant_id: Uuid = row.try_get("grant_id")?;
    if used_at.is_some() {
        // A second use revokes everything issued from the first.
        let family: Option<Uuid> = row.try_get("family_id")?;
        if let Some(family) = family {
            revoke_family(&mut tx, family).await?;
        }
        tx.commit().await?;
        return Err(OAuthError::invalid_grant("the code was already used"));
    }
    let expires_at: DateTime<Utc> = row.try_get("expires_at")?;
    let stored_client: String = row.try_get("client_id")?;
    let stored_redirect: String = row.try_get("redirect_uri")?;
    let challenge: String = row.try_get("code_challenge")?;
    let resource: String = row.try_get("resource")?;
    if expires_at <= Utc::now()
        || stored_client != client_id
        || stored_redirect != redirect_uri
        || !pkce_matches(verifier, &challenge)
    {
        return Err(OAuthError::invalid_grant("the code is not valid"));
    }
    if let Some(requested) = request.resource.as_deref()
        && requested != resource
    {
        return Err(OAuthError::new(400, "invalid_target", "unknown resource"));
    }
    if !grant_usable(&mut tx, grant_id).await? {
        return Err(OAuthError::invalid_grant("the grant is no longer active"));
    }
    let scope: Vec<String> = row.try_get("scope")?;
    let family = Uuid::new_v4();
    sqlx::query(
        "UPDATE penfii.oauth_codes SET used_at = now(), family_id = $2 WHERE code_hash = $1",
    )
    .bind(digest(code))
    .bind(family)
    .execute(&mut *tx)
    .await?;
    let absolute = Utc::now() + config.refresh_absolute_ttl;
    let response = issue_pair(
        &mut tx, config, grant_id, family, &scope, &resource, absolute, None,
    )
    .await?;
    tx.commit().await?;
    Ok(response)
}

async fn grant_usable(conn: &mut PgConnection, grant_id: Uuid) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM penfii.oauth_grants g
                        JOIN penfii.users u ON u.id = g.user_id
                        WHERE g.id = $1 AND g.revoked_at IS NULL AND u.disabled_at IS NULL)",
    )
    .bind(grant_id)
    .fetch_one(conn)
    .await
}

async fn revoke_family(conn: &mut PgConnection, family: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE penfii.oauth_tokens SET revoked_at = now()
         WHERE family_id = $1 AND revoked_at IS NULL",
    )
    .bind(family)
    .execute(conn)
    .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn issue_pair(
    conn: &mut PgConnection,
    config: &OAuthConfig,
    grant_id: Uuid,
    family: Uuid,
    scope: &[String],
    resource: &str,
    absolute: DateTime<Utc>,
    parent: Option<Uuid>,
) -> Result<TokenResponse, OAuthError> {
    let access = random_token("pfa_");
    let refresh = random_token("pfr_");
    let now = Utc::now();
    let refresh_expires = (now + config.refresh_idle_ttl).min(absolute);
    let refresh_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO penfii.oauth_tokens
            (id, grant_id, family_id, kind, token_hash, scope, resource, expires_at, parent_id)
         VALUES ($1, $2, $3, 'access', $4, $5, $6, $7, $8)",
    )
    .bind(Uuid::new_v4())
    .bind(grant_id)
    .bind(family)
    .bind(digest(&access))
    .bind(scope)
    .bind(resource)
    .bind(now + config.access_ttl)
    .bind(parent)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "INSERT INTO penfii.oauth_tokens
            (id, grant_id, family_id, kind, token_hash, scope, resource, expires_at,
             absolute_expires_at, parent_id)
         VALUES ($1, $2, $3, 'refresh', $4, $5, $6, $7, $8, $9)",
    )
    .bind(refresh_id)
    .bind(grant_id)
    .bind(family)
    .bind(digest(&refresh))
    .bind(scope)
    .bind(resource)
    .bind(refresh_expires)
    .bind(absolute)
    .bind(parent)
    .execute(&mut *conn)
    .await?;
    Ok(TokenResponse {
        access_token: access,
        token_type: "Bearer",
        expires_in: config.access_ttl.num_seconds(),
        refresh_token: refresh,
        scope: scope.join(" "),
    })
}

async fn refresh_grant(
    pool: &PgPool,
    config: &OAuthConfig,
    request: TokenRequest,
) -> Result<TokenResponse, OAuthError> {
    let (Some(presented), Some(client_id)) = (
        request.refresh_token.as_deref(),
        request.client_id.as_deref(),
    ) else {
        return Err(OAuthError::invalid_request(
            "refresh_token and client_id are required",
        ));
    };
    let mut tx = pool.begin().await?;
    let row = sqlx::query(
        "SELECT t.id, t.grant_id, t.family_id, t.scope, t.resource, t.expires_at,
                t.absolute_expires_at, t.used_at, t.revoked_at, c.client_id
         FROM penfii.oauth_tokens t
         JOIN penfii.oauth_grants g ON g.id = t.grant_id
         JOIN penfii.oauth_clients c ON c.id = g.client_pk
         WHERE t.token_hash = $1 AND t.kind = 'refresh' FOR UPDATE OF t",
    )
    .bind(digest(presented))
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| OAuthError::invalid_grant("the refresh token is not valid"))?;
    let family: Uuid = row.try_get("family_id")?;
    let used_at: Option<DateTime<Utc>> = row.try_get("used_at")?;
    let revoked_at: Option<DateTime<Utc>> = row.try_get("revoked_at")?;
    if used_at.is_some() {
        // A used refresh token presented again means a copy exists somewhere.
        revoke_family(&mut tx, family).await?;
        tx.commit().await?;
        tracing::warn!(%family, "oauth refresh token reuse; token family revoked");
        return Err(OAuthError::invalid_grant(
            "the refresh token was already used",
        ));
    }
    let expires_at: DateTime<Utc> = row.try_get("expires_at")?;
    let absolute: DateTime<Utc> = row
        .try_get::<Option<DateTime<Utc>>, _>("absolute_expires_at")?
        .unwrap_or(expires_at);
    let stored_client: String = row.try_get("client_id")?;
    let grant_id: Uuid = row.try_get("grant_id")?;
    if revoked_at.is_some()
        || expires_at <= Utc::now()
        || stored_client != client_id
        || !grant_usable(&mut tx, grant_id).await?
    {
        return Err(OAuthError::invalid_grant("the refresh token is not valid"));
    }
    let stored_scope: Vec<String> = row.try_get("scope")?;
    let scope = match request.scope.as_deref() {
        // Refresh can narrow, never widen.
        Some(asked) if !asked.trim().is_empty() => {
            let asked: Vec<String> = asked.split_whitespace().map(str::to_string).collect();
            if asked
                .iter()
                .any(|scope| scope != "offline_access" && !stored_scope.contains(scope))
            {
                return Err(OAuthError::new(
                    400,
                    "invalid_scope",
                    "scope exceeds the original grant",
                ));
            }
            stored_scope
                .iter()
                .filter(|scope| asked.contains(scope))
                .cloned()
                .collect()
        }
        _ => stored_scope,
    };
    let resource: String = row.try_get("resource")?;
    let token_id: Uuid = row.try_get("id")?;
    sqlx::query("UPDATE penfii.oauth_tokens SET used_at = now() WHERE id = $1")
        .bind(token_id)
        .execute(&mut *tx)
        .await?;
    // The old access tokens of this family stop working at rotation.
    sqlx::query(
        "UPDATE penfii.oauth_tokens SET revoked_at = now()
         WHERE family_id = $1 AND kind = 'access' AND revoked_at IS NULL",
    )
    .bind(family)
    .execute(&mut *tx)
    .await?;
    let response = issue_pair(
        &mut tx,
        config,
        grant_id,
        family,
        &scope,
        &resource,
        absolute,
        Some(token_id),
    )
    .await?;
    tx.commit().await?;
    Ok(response)
}

/// RFC 7009: the answer never says whether the token existed. Revoking a refresh token ends its
/// whole family; revoking an access token ends that token.
pub async fn revoke_token(pool: &PgPool, token: &str) -> Result<(), CoreError> {
    let row =
        sqlx::query("SELECT id, kind, family_id FROM penfii.oauth_tokens WHERE token_hash = $1")
            .bind(digest(token))
            .fetch_optional(pool)
            .await?;
    if let Some(row) = row {
        let kind: String = row.try_get("kind")?;
        if kind == "refresh" {
            let family: Uuid = row.try_get("family_id")?;
            let mut conn = pool.acquire().await?;
            revoke_family(&mut conn, family).await?;
        } else {
            let id: Uuid = row.try_get("id")?;
            sqlx::query("UPDATE penfii.oauth_tokens SET revoked_at = now() WHERE id = $1 AND revoked_at IS NULL")
                .bind(id)
                .execute(pool)
                .await?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Bearer authentication (Resource Server)
// ---------------------------------------------------------------------------------------------

/// The person acting through a client. There is no MCP user.
#[derive(Debug, Clone)]
pub struct Principal {
    pub user_id: Uuid,
    pub grant_id: Uuid,
    pub client_pk: Uuid,
    pub client_id: String,
    pub client_name: String,
    pub scopes: Vec<String>,
    pub workspace_filter: Option<Vec<Uuid>>,
}

/// Not expired, not revoked, grant live, user enabled, resource matches. `None` for any
/// failure; the caller answers 401.
pub async fn authenticate_bearer(
    pool: &PgPool,
    config: &OAuthConfig,
    token: &str,
) -> Result<Option<Principal>, CoreError> {
    if !token.starts_with("pfa_") {
        return Ok(None);
    }
    let row = sqlx::query(
        "SELECT t.id, t.scope, g.id AS grant_id, g.user_id, g.client_pk, g.workspace_ids,
                c.client_id, c.client_name, t.last_used_at
         FROM penfii.oauth_tokens t
         JOIN penfii.oauth_grants g ON g.id = t.grant_id
         JOIN penfii.oauth_clients c ON c.id = g.client_pk
         JOIN penfii.users u ON u.id = g.user_id
         WHERE t.token_hash = $1 AND t.kind = 'access' AND t.revoked_at IS NULL
           AND t.expires_at > now() AND g.revoked_at IS NULL AND u.disabled_at IS NULL
           AND t.resource = $2",
    )
    .bind(digest(token))
    .bind(config.resource())
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let token_id: Uuid = row.try_get("id")?;
    let grant_id: Uuid = row.try_get("grant_id")?;
    let last_used: Option<DateTime<Utc>> = row.try_get("last_used_at")?;
    if last_used.is_none_or(|at| Utc::now() - at > Duration::seconds(60)) {
        let _ = sqlx::query("UPDATE penfii.oauth_tokens SET last_used_at = now() WHERE id = $1")
            .bind(token_id)
            .execute(pool)
            .await;
        let _ = sqlx::query("UPDATE penfii.oauth_grants SET last_used_at = now() WHERE id = $1")
            .bind(grant_id)
            .execute(pool)
            .await;
    }
    Ok(Some(Principal {
        user_id: row.try_get("user_id")?,
        grant_id,
        client_pk: row.try_get("client_pk")?,
        client_id: row.try_get("client_id")?,
        client_name: row.try_get("client_name")?,
        scopes: row.try_get("scope")?,
        workspace_filter: row.try_get("workspace_ids")?,
    }))
}

// ---------------------------------------------------------------------------------------------
// Grants ("connected apps")
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantInfo {
    pub id: Uuid,
    pub client_name: String,
    pub client_id: String,
    pub scopes: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

pub async fn list_grants(pool: &PgPool, user_id: Uuid) -> Result<Vec<GrantInfo>, CoreError> {
    let rows = sqlx::query(
        "SELECT g.id, g.scope, g.created_at, g.last_used_at, c.client_name, c.client_id
         FROM penfii.oauth_grants g JOIN penfii.oauth_clients c ON c.id = g.client_pk
         WHERE g.user_id = $1 AND g.revoked_at IS NULL ORDER BY g.created_at DESC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|row| {
            Ok(GrantInfo {
                id: row.try_get("id")?,
                client_name: row.try_get("client_name")?,
                client_id: row.try_get("client_id")?,
                scopes: row.try_get("scope")?,
                created_at: row.try_get("created_at")?,
                last_used_at: row.try_get("last_used_at")?,
            })
        })
        .collect::<Result<_, sqlx::Error>>()
        .map_err(Into::into)
}

/// Revokes one grant and, in the same transaction, every token of it.
pub async fn revoke_grant(
    pool: &PgPool,
    user_id: Uuid,
    grant_id: Uuid,
    reason: &str,
) -> Result<(), CoreError> {
    let mut tx = pool.begin().await?;
    let done = sqlx::query(
        "UPDATE penfii.oauth_grants SET revoked_at = now(), revoked_reason = $3
         WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL",
    )
    .bind(grant_id)
    .bind(user_id)
    .bind(reason)
    .execute(&mut *tx)
    .await?;
    if done.rows_affected() != 1 {
        return Err(CoreError::NotFound);
    }
    sqlx::query(
        "UPDATE penfii.oauth_tokens SET revoked_at = now() WHERE grant_id = $1 AND revoked_at IS NULL",
    )
    .bind(grant_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    policy::audit(
        pool,
        policy::AuditRow {
            user_id,
            workspace_id: None,
            action_code: "oauth.grant_revoked",
            outcome_code: "OK",
            target_kind: "oauth_grant",
            target_id: Some(grant_id),
            data: json!({"reason": reason}),
        },
    )
    .await;
    Ok(())
}

/// "Disconnect all apps": its own action, needs recent authentication at the route.
pub async fn revoke_all_grants(pool: &PgPool, user_id: Uuid) -> Result<u64, CoreError> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "UPDATE penfii.oauth_tokens SET revoked_at = now() WHERE revoked_at IS NULL AND grant_id IN
            (SELECT id FROM penfii.oauth_grants WHERE user_id = $1 AND revoked_at IS NULL)",
    )
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    let done = sqlx::query(
        "UPDATE penfii.oauth_grants SET revoked_at = now(), revoked_reason = 'user_all'
         WHERE user_id = $1 AND revoked_at IS NULL",
    )
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    policy::audit(
        pool,
        policy::AuditRow {
            user_id,
            workspace_id: None,
            action_code: "oauth.grants_revoked_all",
            outcome_code: "OK",
            target_kind: "user",
            target_id: Some(user_id),
            data: json!({"count": done.rows_affected()}),
        },
    )
    .await;
    Ok(done.rows_affected())
}

// ---------------------------------------------------------------------------------------------
// Dynamic Client Registration (compatibility only)
// ---------------------------------------------------------------------------------------------

/// Public clients only, behind `PENFII_MCP_DCR_ENABLED`, bounded in number and age.
pub async fn register_client(
    pool: &PgPool,
    config: &OAuthConfig,
    body: &Value,
) -> Result<Value, OAuthError> {
    let invalid = |message: &str| OAuthError::new(400, "invalid_client_metadata", message);
    if !config.dcr_enabled {
        return Err(OAuthError::new(
            404,
            "invalid_request",
            "registration is not enabled",
        ));
    }
    let uris: Vec<String> = body
        .get("redirect_uris")
        .and_then(Value::as_array)
        .map(|all| {
            all.iter()
                .filter_map(|uri| uri.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if uris.is_empty() || uris.len() > 10 || !uris.iter().all(|uri| client::valid_redirect_uri(uri))
    {
        return Err(invalid(
            "redirect_uris must be https or loopback http addresses",
        ));
    }
    if let Some(method) = body
        .get("token_endpoint_auth_method")
        .and_then(Value::as_str)
        && method != "none"
    {
        return Err(invalid("only token_endpoint_auth_method none is supported"));
    }
    let name: String = body
        .get("client_name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("Unnamed application")
        .chars()
        .take(100)
        .collect();
    let recent: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.oauth_clients
         WHERE kind = 'dcr' AND fetched_at > now() - interval '1 hour'",
    )
    .fetch_one(pool)
    .await?;
    if recent >= 100 {
        return Err(OAuthError::new(
            429,
            "temporarily_unavailable",
            "too many registrations",
        ));
    }
    // Unused registrations expire.
    let _ = sqlx::query(
        "DELETE FROM penfii.oauth_clients c WHERE c.kind = 'dcr' AND c.fetched_at < now() - interval '7 days'
           AND NOT EXISTS (SELECT 1 FROM penfii.oauth_grants g WHERE g.client_pk = c.id)
           AND NOT EXISTS (SELECT 1 FROM penfii.oauth_authorization_requests r WHERE r.client_pk = c.id)",
    )
    .execute(pool)
    .await;
    let client_id = random_token("pfi_");
    sqlx::query(
        "INSERT INTO penfii.oauth_clients (id, client_id, kind, client_name, redirect_uris)
         VALUES ($1, $2, 'dcr', $3, $4)",
    )
    .bind(Uuid::new_v4())
    .bind(&client_id)
    .bind(&name)
    .bind(json!(uris))
    .execute(pool)
    .await?;
    Ok(json!({
        "client_id": client_id,
        "client_name": name,
        "redirect_uris": uris,
        "token_endpoint_auth_method": "none",
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_s256_check() {
        // RFC 7636 appendix B.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert!(pkce_matches(verifier, challenge));
        assert!(!pkce_matches(
            verifier,
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cX"
        ));
        assert!(!pkce_matches("short", challenge));
    }

    #[test]
    fn tokens_have_scanner_prefixes_and_enough_entropy() {
        let access = random_token("pfa_");
        assert!(access.starts_with("pfa_"));
        assert_eq!(access.len(), 4 + 43);
        assert_ne!(access, random_token("pfa_"));
    }
}
