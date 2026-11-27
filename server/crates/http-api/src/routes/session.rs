//! Sign-in, sessions, the signed-in person's profile and avatar, and linked identities.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use penfii_contracts::LoginRequest;
use penfii_core::CoreError;
use penfii_core::identity;
use uuid::Uuid;

use crate::AppState;
use crate::error::ApiError;
use crate::extract::{self, Actor, Body, CurrentUser, Session};

use axum::Router;
use axum::routing::{delete, get, patch, post, put};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/session/login", post(login))
        .route("/v1/session/logout", post(logout))
        .route(
            "/v1/auth/password-reset/request",
            post(password_reset_request),
        )
        .route(
            "/v1/auth/password-reset/confirm",
            post(password_reset_confirm),
        )
        .route("/v1/me", get(me).patch(update_me))
        .route(
            "/v1/me/email-change",
            get(email_change_pending).post(email_change_request),
        )
        .route("/v1/auth/email-change/confirm", post(email_change_confirm))
        .route("/v1/me/profile", patch(update_public_profile))
        .route("/v1/me/avatar", put(put_avatar).delete(delete_avatar))
        .route("/v1/users/{id}/avatar", get(get_avatar))
        .route("/v1/me/reauth/password", post(reauth_password))
        .route(
            "/v1/me/reauth/{provider}/start",
            post(reauth_provider_start),
        )
        .route("/v1/me/mfa", get(mfa_status))
        .route("/v1/me/mfa/setup", post(mfa_setup))
        .route("/v1/me/mfa/enable", post(mfa_enable))
        .route("/v1/me/mfa/disable", post(mfa_disable))
        .route("/v1/me/mfa/recovery-codes", post(mfa_recovery_codes))
        .route("/v1/me/identities", get(list_identities))
        .route("/v1/me/identities/{provider}", delete(unlink_identity))
        .route("/v1/me/password", delete(remove_password))
        .route("/v1/auth/providers", get(auth_providers))
        .route("/v1/auth/{provider}/start", post(auth_start))
        .route("/v1/auth/{provider}/callback", get(auth_callback))
        .route(
            "/v1/me/sessions",
            get(list_sessions).delete(revoke_other_sessions),
        )
        .route("/v1/me/sessions/{id}", delete(revoke_session))
}

async fn login(
    State(state): State<AppState>,
    peer: Option<axum::extract::Extension<axum::extract::ConnectInfo<std::net::SocketAddr>>>,
    headers: HeaderMap,
    Body(body): Body<LoginRequest>,
) -> Result<Response, ApiError> {
    let address = extract::client_address(peer.map(|found| found.0.0), &headers);
    let device = extract::device_label(&headers);
    let result = identity::login_with_code(
        &state.pool,
        &state.limits,
        &body.email,
        &body.password,
        body.code.as_deref(),
        body.keep_signed_in,
        address.as_deref(),
        device.as_deref(),
    )
    .await?;
    let cookie = extract::session_cookie(&result.token, result.ttl_seconds, state.cookie_secure);
    let mut response = Json(result.user).into_response();
    set_cookie(&mut response, &cookie)?;
    Ok(response)
}

#[derive(serde::Deserialize)]
struct PasswordResetRequest {
    email: String,
}

/// Asks for a password reset mail. Every well-formed request gets the same answer (202, no body) at
/// the same speed whether or not the account exists, is disabled, or has a password at all: the
/// request is counted and the rest happens in a background task. It answers 404 while the server
/// sends no e-mail (for every request alike, so that says nothing about accounts either).
async fn password_reset_request(
    State(state): State<AppState>,
    peer: Option<axum::extract::Extension<axum::extract::ConnectInfo<std::net::SocketAddr>>>,
    headers: HeaderMap,
    Body(body): Body<PasswordResetRequest>,
) -> Result<Response, ApiError> {
    let Some(mailer) = state.mailer.clone() else {
        return Err(CoreError::NotFound.into());
    };
    let (_, email_key) = identity::normalize_email(&body.email)?;
    let address = extract::client_address(peer.map(|found| found.0.0), &headers);
    let admitted = identity::admit_reset_request(
        &state.pool,
        &state.limits.login_throttle,
        &email_key,
        address.as_deref(),
    )
    .await?;
    if admitted {
        let pool = state.pool.clone();
        tokio::spawn(async move {
            identity::process_reset_request(&pool, &mailer, &email_key).await;
        });
    }
    Ok(StatusCode::ACCEPTED.into_response())
}

#[derive(serde::Deserialize)]
struct PasswordResetConfirm {
    token: String,
    password: String,
}

/// Sets a new password with a reset token (from the mailed link) and ends every session of the account.
async fn password_reset_confirm(
    State(state): State<AppState>,
    Body(body): Body<PasswordResetConfirm>,
) -> Result<Response, ApiError> {
    if state.mailer.is_none() {
        return Err(CoreError::NotFound.into());
    }
    identity::confirm_password_reset(&state.pool, &body.token, &body.password).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

fn set_cookie(response: &mut Response, cookie: &str) -> Result<(), ApiError> {
    let value =
        HeaderValue::from_str(cookie).map_err(|_| CoreError::Internal("cookie header".into()))?;
    response.headers_mut().insert(header::SET_COOKIE, value);
    Ok(())
}

async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, ApiError> {
    if let Some(token) = extract::session_token(&headers) {
        identity::logout(&state.pool, &token).await?;
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    let cookie = extract::clear_cookie(state.cookie_secure);
    set_cookie(&mut response, &cookie)?;
    Ok(response)
}

async fn me(CurrentUser(user): CurrentUser) -> Json<penfii_contracts::User> {
    Json(user)
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateProfileRequest {
    display_name: String,
    email: String,
    #[serde(default)]
    current_password: Option<String>,
    #[serde(default)]
    new_password: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublicProfileRequest {
    display_name: String,
    #[serde(default)]
    job_title: Option<String>,
    #[serde(default)]
    time_zone: Option<String>,
    #[serde(default)]
    status_emoji: Option<String>,
    #[serde(default)]
    status_text: Option<String>,
    #[serde(default)]
    status_clear_at: Option<DateTime<Utc>>,
}

/// Name, title, time zone and status. These are not credentials, so they do not ask for a fresh sign-in.
async fn update_public_profile(
    State(state): State<AppState>,
    Actor(user_id): Actor,
    Body(body): Body<PublicProfileRequest>,
) -> Result<Response, ApiError> {
    let updated = identity::update_public_profile(
        &state.pool,
        user_id,
        identity::PublicProfile {
            display_name: body.display_name,
            job_title: body.job_title,
            time_zone: body.time_zone,
            status_emoji: body.status_emoji,
            status_text: body.status_text,
            status_clear_at: body.status_clear_at,
        },
    )
    .await?;
    Ok(Json(updated).into_response())
}

fn object_store(state: &AppState) -> Result<&penfii_core::files::ObjectStore, ApiError> {
    state.objects.as_ref().ok_or_else(|| {
        CoreError::conflict("storage_unconfigured", "object storage is not configured").into()
    })
}

async fn put_avatar(
    State(state): State<AppState>,
    Actor(user_id): Actor,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Response, ApiError> {
    let mime = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    identity::set_avatar(&state.pool, object_store(&state)?, user_id, body, mime).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn delete_avatar(
    State(state): State<AppState>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    identity::clear_avatar(&state.pool, object_store(&state)?, user_id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn get_avatar(
    State(state): State<AppState>,
    Actor(actor): Actor,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let (bytes, mime) =
        identity::read_avatar(&state.pool, object_store(&state)?, actor, id).await?;
    let content_type = HeaderValue::from_str(&mime)
        .map_err(|_| ApiError::from(CoreError::Internal("avatar mime".into())))?;
    Ok((
        [
            (header::CONTENT_TYPE, content_type),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("private, max-age=60"),
            ),
        ],
        bytes,
    )
        .into_response())
}

async fn update_me(
    State(state): State<AppState>,
    Session(session): Session,
    Body(body): Body<UpdateProfileRequest>,
) -> Result<Response, ApiError> {
    let updated = identity::update_profile(
        &state.pool,
        &session,
        identity::ProfileUpdate {
            display_name: body.display_name,
            email: body.email,
            current_password: body.current_password,
            new_password: body.new_password,
        },
    )
    .await?;
    Ok(Json(updated).into_response())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EmailChangeRequest {
    email: String,
}

/// Mails a confirmation link to a new address. The account keeps its address until the link is used.
/// Needs a recent authentication and a server that sends e-mail (`409 email_change_unavailable`).
async fn email_change_request(
    State(state): State<AppState>,
    Session(session): Session,
    Body(body): Body<EmailChangeRequest>,
) -> Result<Response, ApiError> {
    let pending =
        identity::request_email_change(&state.pool, state.mailer.as_deref(), &session, &body.email)
            .await?;
    Ok((StatusCode::ACCEPTED, Json(pending)).into_response())
}

/// The change that is waiting for its link: `null` when there is none.
async fn email_change_pending(
    State(state): State<AppState>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    Ok(Json(identity::pending_email_change(&state.pool, user_id).await?).into_response())
}

#[derive(serde::Deserialize)]
struct EmailChangeConfirm {
    token: String,
}

/// Uses the link mailed to the new address. The link is the proof; no session is needed (it is often
/// opened on another device).
async fn email_change_confirm(
    State(state): State<AppState>,
    Body(body): Body<EmailChangeConfirm>,
) -> Result<Response, ApiError> {
    identity::confirm_email_change(&state.pool, state.mailer.clone(), &body.token).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReauthPasswordRequest {
    password: String,
}

async fn reauth_password(
    State(state): State<AppState>,
    Session(session): Session,
    Body(body): Body<ReauthPasswordRequest>,
) -> Result<Response, ApiError> {
    identity::reauth_with_password(&state.pool, &session, &body.password).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// Two-step verification of the signed-in person: whether it is on, and how many recovery codes are left.
async fn mfa_status(
    State(state): State<AppState>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    Ok(Json(identity::mfa_status(&state.pool, user_id).await?).into_response())
}

/// Starts the setup (needs a recent authentication): a new secret that protects nothing until `enable`.
async fn mfa_setup(
    State(state): State<AppState>,
    Session(session): Session,
) -> Result<Response, ApiError> {
    Ok(secret_answer(Json(
        identity::begin_mfa_setup(&state.pool, &session).await?,
    )))
}

/// An answer that carries a secret (an authenticator secret, recovery codes): no cache keeps it.
fn secret_answer(json: impl IntoResponse) -> Response {
    ([(header::CACHE_CONTROL, "no-store")], json).into_response()
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MfaCodeRequest {
    code: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct RecoveryCodes {
    recovery_codes: Vec<String>,
}

/// Turns it on with a current code of the new secret; answers with the recovery codes, once.
async fn mfa_enable(
    State(state): State<AppState>,
    Session(session): Session,
    Body(body): Body<MfaCodeRequest>,
) -> Result<Response, ApiError> {
    let recovery_codes = identity::confirm_mfa_setup(&state.pool, &session, &body.code).await?;
    Ok(secret_answer(Json(RecoveryCodes { recovery_codes })))
}

/// Turns it off: a recent authentication and a current authenticator or recovery code.
async fn mfa_disable(
    State(state): State<AppState>,
    Session(session): Session,
    Body(body): Body<MfaCodeRequest>,
) -> Result<Response, ApiError> {
    identity::disable_mfa(&state.pool, &session, &body.code).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// New recovery codes in place of the old ones: same requirements as turning it off.
async fn mfa_recovery_codes(
    State(state): State<AppState>,
    Session(session): Session,
    Body(body): Body<MfaCodeRequest>,
) -> Result<Response, ApiError> {
    let recovery_codes =
        identity::regenerate_recovery_codes(&state.pool, &session, &body.code).await?;
    Ok(secret_answer(Json(RecoveryCodes { recovery_codes })))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthStartRequest {
    #[serde(default)]
    intent: Option<String>,
    #[serde(default)]
    invitation_token: Option<String>,
    #[serde(default)]
    return_to: Option<String>,
    /// "Keep me signed in", for `intent: login` only.
    #[serde(default)]
    keep_signed_in: bool,
}

/// The sign-in methods on offer, and whether the server offers a long session (the "keep me signed
/// in" choice) with its two limits, so the sign-in page can show the option and say what it means.
async fn auth_providers(State(state): State<AppState>) -> Result<Response, ApiError> {
    let long = state.limits.long_session;
    Ok(Json(serde_json::json!({
        "items": identity::offered(&state.providers),
        "longSession": {
            "enabled": long.enabled,
            "maxAgeSeconds": long.max_age_seconds,
            "idleSeconds": long.effective_idle_seconds(),
        },
    }))
    .into_response())
}

async fn auth_start(
    State(state): State<AppState>,
    peer: Option<axum::extract::Extension<axum::extract::ConnectInfo<std::net::SocketAddr>>>,
    Path(provider): Path<String>,
    headers: HeaderMap,
    session: Option<Session>,
    Body(body): Body<AuthStartRequest>,
) -> Result<Response, ApiError> {
    let session = session.map(|Session(session)| session);
    let intent = match body.intent.as_deref().unwrap_or("login") {
        "login" => identity::Intent::Login,
        "invite" => identity::Intent::Invite,
        "link" => identity::Intent::Link,
        _ => return Err(ApiError::validation("intent is invalid")),
    };
    let address = extract::client_address(peer.map(|found| found.0.0), &headers);
    let started = identity::start(
        &state.pool,
        &state.providers,
        identity::StartInput {
            provider_key: &provider,
            intent,
            session: session.as_ref(),
            invitation_token: body.invitation_token.as_deref(),
            return_to: body.return_to.as_deref(),
            address: address.as_deref(),
            throttle: &state.limits.login_throttle,
            keep_signed_in: body.keep_signed_in,
        },
    )
    .await?;
    started_response(&state, started)
}

/// The provider URL, and the cookie that ties the attempt to this browser.
fn started_response(
    state: &AppState,
    started: identity::StartedAttempt,
) -> Result<Response, ApiError> {
    let mut response = Json(serde_json::json!({"url": started.url})).into_response();
    let cookie = extract::auth_binding_cookie(
        &started.browser_binding,
        identity::ATTEMPT_COOKIE_SECONDS,
        state.cookie_secure,
    );
    append_cookie(&mut response, &cookie)?;
    Ok(response)
}

fn append_cookie(response: &mut Response, cookie: &str) -> Result<(), ApiError> {
    let value =
        HeaderValue::from_str(cookie).map_err(|_| CoreError::Internal("cookie header".into()))?;
    response.headers_mut().append(header::SET_COOKIE, value);
    Ok(())
}

async fn reauth_provider_start(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    Session(session): Session,
    Body(body): Body<AuthStartRequest>,
) -> Result<Response, ApiError> {
    let started = identity::start(
        &state.pool,
        &state.providers,
        identity::StartInput {
            provider_key: &provider,
            intent: identity::Intent::Reauth,
            session: Some(&session),
            invitation_token: None,
            return_to: body.return_to.as_deref(),
            address: None,
            throttle: &state.limits.login_throttle,
            keep_signed_in: false,
        },
    )
    .await?;
    started_response(&state, started)
}

#[derive(serde::Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

/// Always a redirect to a relative path; nothing from the query is echoed
/// or logged.
async fn auth_callback(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    headers: HeaderMap,
    extract::Q(query): extract::Q<CallbackQuery>,
) -> Result<Response, ApiError> {
    let session_token = extract::session_token(&headers);
    let outcome = identity::callback(
        &state.pool,
        &state.limits,
        &state.providers,
        identity::CallbackInput {
            provider_key: &provider,
            code: query.code.as_deref(),
            state: query.state.as_deref(),
            error: query.error.as_deref(),
            session_token: session_token.as_deref(),
            device: extract::device_label(&headers).as_deref(),
            browser_binding: extract::auth_binding(&headers).as_deref(),
        },
    )
    .await;
    let mut response = Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(header::LOCATION, &outcome.redirect)
        .header(header::REFERRER_POLICY, "no-referrer")
        .header(header::CACHE_CONTROL, "no-store")
        .body(axum::body::Body::empty())
        .map_err(|_| CoreError::Internal("redirect".into()))?;
    let cookie = match (&outcome.new_session, outcome.clear_session) {
        (Some((token, ttl)), _) => Some(extract::session_cookie(token, *ttl, state.cookie_secure)),
        (None, true) => Some(extract::clear_cookie(state.cookie_secure)),
        _ => None,
    };
    if let Some(cookie) = cookie {
        set_cookie(&mut response, &cookie)?;
    }
    // The attempt is over, whatever came of it: the binding goes with it.
    let done = extract::auth_binding_cookie("", 0, state.cookie_secure);
    append_cookie(&mut response, &done)?;
    Ok(response)
}

async fn list_identities(
    State(state): State<AppState>,
    Session(session): Session,
) -> Result<Response, ApiError> {
    let has_password = identity::reauth_methods(&state.pool, session.user.id)
        .await?
        .iter()
        .any(|method| method == "password");
    Ok(Json(serde_json::json!({
        "items": identity::list_identities(&state.pool, session.user.id).await?,
        "hasPassword": has_password,
        "providers": identity::offered(&state.providers),
    }))
    .into_response())
}

async fn unlink_identity(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    Session(session): Session,
) -> Result<Response, ApiError> {
    identity::unlink_identity(&state.pool, &session, &provider).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn remove_password(
    State(state): State<AppState>,
    Session(session): Session,
) -> Result<Response, ApiError> {
    identity::remove_password(&state.pool, &session).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn list_sessions(
    State(state): State<AppState>,
    Session(session): Session,
) -> Result<Response, ApiError> {
    let items = identity::list_sessions(&state.pool, session.user.id, session.id).await?;
    Ok(Json(serde_json::json!({
        "items": items,
        "reauthMethods": identity::reauth_methods(&state.pool, session.user.id).await?,
        "reauthWindowSeconds": identity::reauth_window_seconds(),
    }))
    .into_response())
}

async fn revoke_session(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Session(session): Session,
) -> Result<Response, ApiError> {
    identity::revoke_session(&state.pool, session.user.id, id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn revoke_other_sessions(
    State(state): State<AppState>,
    Session(session): Session,
) -> Result<Response, ApiError> {
    let revoked = identity::revoke_other_sessions(&state.pool, &session).await?;
    Ok(Json(serde_json::json!({"revoked": revoked})).into_response())
}
