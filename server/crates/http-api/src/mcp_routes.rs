//! MCP OAuth Authorization Server endpoints, the consent API, connected apps, the Workspace MCP
//! Policy API and the `/mcp` Resource Server endpoint.
//!
//! Everything answers 404 while `PENFII_MCP_ENABLED` is off. `/oauth/token`, `/oauth/revoke`,
//! `/oauth/register` and `/mcp` never read the session cookie; the consent decision
//! and grant management do, and keep the CSRF header rule.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Form, FromRequestParts, Path, State};
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use http::{HeaderMap, HeaderValue, StatusCode, header};
use penfii_core::oauth::{self, AuthorizeParams, AuthorizeResult, Decision, TokenRequest};
use penfii_mcp::{McpContext, Mirrored};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

use crate::AppState;
use crate::error::ApiError;
use crate::extract::{Actor, Body, Q, RawJson, RecentActor, Session};
use crate::origin::{self, RequestOrigin};

use axum::Router;
use axum::routing::{delete, get, post};

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/.well-known/oauth-protected-resource",
            get(protected_resource_metadata),
        )
        .route(
            "/.well-known/oauth-protected-resource/mcp",
            get(protected_resource_metadata),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            get(authorization_server_metadata),
        )
        .route(
            "/.well-known/openid-configuration",
            get(authorization_server_metadata),
        )
        .route("/oauth/authorize", get(authorize))
        .route("/oauth/token", post(token))
        .route("/oauth/revoke", post(revoke))
        .route("/oauth/register", post(register))
        .route("/v1/oauth/requests/{id}", get(consent_request))
        .route("/v1/oauth/requests/{id}/decision", post(consent_decision))
        .route(
            "/v1/me/oauth-grants",
            get(list_grants).delete(revoke_all_grants),
        )
        .route("/v1/me/oauth-grants/{id}", delete(revoke_grant))
        .route(
            "/v1/workspaces/{id}/mcp-policy",
            get(get_policy).put(put_policy),
        )
        .route("/mcp", post(mcp_post).get(mcp_other).delete(mcp_other))
}

const MAX_MCP_BODY_BYTES: usize = 1024 * 1024;

/// The MCP context. Every route in this file takes it first, so all of them answer an empty
/// `404` while MCP is off, before anything else about the request is looked at.
pub struct Mcp(Arc<McpContext>);

impl FromRequestParts<AppState> for Mcp {
    type Rejection = StatusCode;

    async fn from_request_parts(_: &mut Parts, state: &AppState) -> Result<Self, StatusCode> {
        state.mcp.clone().map(Self).ok_or(StatusCode::NOT_FOUND)
    }
}

fn no_store(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    response
}

fn public_metadata(value: Value) -> Response {
    let mut response = Json(value).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=300"),
    );
    response
}

async fn protected_resource_metadata(Mcp(ctx): Mcp) -> Response {
    public_metadata(ctx.oauth.protected_resource_metadata())
}

async fn authorization_server_metadata(Mcp(ctx): Mcp) -> Response {
    public_metadata(ctx.oauth.authorization_server_metadata())
}

fn oauth_error(error: oauth::OAuthError) -> Response {
    let status = StatusCode::from_u16(error.status).unwrap_or(StatusCode::BAD_REQUEST);
    no_store((status, Json(error.body())).into_response())
}

fn error_page(message: &str) -> Response {
    let escaped = message
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let html = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Penfii</title></head>\
         <body style=\"font-family:system-ui;max-width:32rem;margin:4rem auto;padding:0 1rem\">\
         <h1>Cannot continue</h1><p>{escaped}</p><p>Close this window and try again from the application.</p></body></html>"
    );
    let mut response = (StatusCode::BAD_REQUEST, axum::response::Html(html)).into_response();
    response.headers_mut().insert(
        header::HeaderName::from_static("referrer-policy"),
        HeaderValue::from_static("no-referrer"),
    );
    // The page styles itself with a style attribute and loads nothing.
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'none'; style-src 'unsafe-inline'; frame-ancestors 'none'",
        ),
    );
    no_store(response)
}

fn redirect(location: &str) -> Response {
    let mut response = StatusCode::FOUND.into_response();
    if let Ok(value) = HeaderValue::from_str(location) {
        response.headers_mut().insert(header::LOCATION, value);
    }
    response.headers_mut().insert(
        header::HeaderName::from_static("referrer-policy"),
        HeaderValue::from_static("no-referrer"),
    );
    no_store(response)
}

#[derive(Deserialize, Default)]
pub struct AuthorizeQuery {
    response_type: Option<String>,
    client_id: Option<String>,
    redirect_uri: Option<String>,
    state: Option<String>,
    code_challenge: Option<String>,
    code_challenge_method: Option<String>,
    scope: Option<String>,
    resource: Option<String>,
}

async fn authorize(
    State(state): State<AppState>,
    Mcp(ctx): Mcp,
    Q(query): Q<AuthorizeQuery>,
) -> Response {
    let params = AuthorizeParams {
        response_type: query.response_type,
        client_id: query.client_id,
        redirect_uri: query.redirect_uri,
        state: query.state,
        code_challenge: query.code_challenge,
        code_challenge_method: query.code_challenge_method,
        scope: query.scope,
        resource: query.resource,
    };
    match oauth::server::begin_authorization(&state.pool, &ctx.oauth, params).await {
        Ok(AuthorizeResult::Consent(id)) => redirect(&format!("/consent?request={id}")),
        Ok(AuthorizeResult::Redirect(url)) => redirect(&url),
        Ok(AuthorizeResult::Page(message)) => error_page(&message),
        Err(error) => {
            tracing::error!(?error, "oauth authorize failed");
            error_page("The request could not be processed.")
        }
    }
}

async fn consent_request(
    State(state): State<AppState>,
    _: Mcp,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    let view = oauth::server::consent_view(&state.pool, user_id, id).await?;
    Ok(no_store(Json(view).into_response()))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DecisionBody {
    approve: bool,
    #[serde(default)]
    scopes: Vec<String>,
}

async fn consent_decision(
    State(state): State<AppState>,
    Mcp(ctx): Mcp,
    Path(id): Path<Uuid>,
    Session(session): Session,
    Body(body): Body<DecisionBody>,
) -> Result<Response, ApiError> {
    let decision = if body.approve {
        Decision::Approve {
            scopes: body.scopes,
        }
    } else {
        Decision::Deny
    };
    let redirect_to =
        oauth::server::decide(&state.pool, &ctx.oauth, &session, id, decision).await?;
    Ok(no_store(
        Json(json!({"redirectTo": redirect_to})).into_response(),
    ))
}

#[derive(Deserialize)]
pub struct TokenForm {
    grant_type: Option<String>,
    code: Option<String>,
    redirect_uri: Option<String>,
    client_id: Option<String>,
    code_verifier: Option<String>,
    refresh_token: Option<String>,
    scope: Option<String>,
    resource: Option<String>,
}

async fn token(
    State(state): State<AppState>,
    Mcp(ctx): Mcp,
    Form(form): Form<TokenForm>,
) -> Response {
    let request = TokenRequest {
        grant_type: form.grant_type,
        code: form.code,
        redirect_uri: form.redirect_uri,
        client_id: form.client_id,
        code_verifier: form.code_verifier,
        refresh_token: form.refresh_token,
        scope: form.scope,
        resource: form.resource,
    };
    match oauth::server::token(&state.pool, &ctx.oauth, request).await {
        Ok(response) => no_store(Json(response).into_response()),
        Err(error) => oauth_error(error),
    }
}

#[derive(Deserialize)]
pub struct RevokeForm {
    token: Option<String>,
}

async fn revoke(State(state): State<AppState>, _: Mcp, Form(form): Form<RevokeForm>) -> Response {
    if let Some(token) = form.token.as_deref() {
        // RFC 7009: the answer never says whether the token existed.
        let _ = oauth::server::revoke_token(&state.pool, token).await;
    }
    no_store(StatusCode::OK.into_response())
}

async fn register(
    State(state): State<AppState>,
    Mcp(ctx): Mcp,
    RawJson(body): RawJson,
) -> Response {
    match oauth::server::register_client(&state.pool, &ctx.oauth, &body).await {
        Ok(value) => no_store((StatusCode::CREATED, Json(value)).into_response()),
        Err(error) => oauth_error(error),
    }
}

// ---- connected apps -------------------------------------------------------------------------

async fn list_grants(
    State(state): State<AppState>,
    _: Mcp,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    let items = oauth::server::list_grants(&state.pool, user_id).await?;
    Ok(Json(json!({"items": items})).into_response())
}

async fn revoke_grant(
    State(state): State<AppState>,
    _: Mcp,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    oauth::server::revoke_grant(&state.pool, user_id, id, "user").await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn revoke_all_grants(
    State(state): State<AppState>,
    _: Mcp,
    RecentActor(user_id): RecentActor,
) -> Result<Response, ApiError> {
    oauth::server::revoke_all_grants(&state.pool, user_id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

// ---- Workspace MCP Policy -------------------------------------------------------------------

async fn get_policy(
    State(state): State<AppState>,
    Mcp(ctx): Mcp,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    let found = oauth::policy::get(&state.pool, ctx.oauth.policy_defaults, user_id, id).await?;
    Ok(Json(found).into_response())
}

async fn put_policy(
    State(state): State<AppState>,
    Mcp(ctx): Mcp,
    Path(id): Path<Uuid>,
    Session(session): Session,
    Body(input): Body<oauth::PolicyInput>,
) -> Result<Response, ApiError> {
    let updated =
        oauth::policy::set(&state.pool, ctx.oauth.policy_defaults, &session, id, input).await?;
    Ok(Json(updated).into_response())
}

// ---- /mcp -----------------------------------------------------------------------------------

/// `/mcp` trusts the public URL and `PENFII_MCP_ALLOWED_ORIGINS`, and not the request's `Host` (the
/// WebSocket rule); the header is read by the shared [`origin::request_origin`]. No `Origin` is
/// allowed (servers and command-line clients send none; the bearer token still decides), an
/// unreadable one is refused.
fn origin_allowed(ctx: &McpContext, headers: &HeaderMap) -> bool {
    match origin::request_origin(headers) {
        RequestOrigin::Absent => true,
        RequestOrigin::Unreadable => false,
        RequestOrigin::Present(origin) => {
            origin == origin::normalize(&ctx.oauth.public_url)
                || ctx
                    .oauth
                    .allowed_origins
                    .iter()
                    .any(|allowed| origin::normalize(allowed) == origin)
        }
    }
}

fn host_allowed(ctx: &McpContext, headers: &HeaderMap) -> bool {
    // A reverse proxy rewrites `Host` to its upstream; it then says what the client asked for in
    // `X-Forwarded-Host`. Either one naming the public host is enough: this check only rejects
    // DNS-rebinding requests, which carry neither. The public URL, not any header, builds every
    // absolute URL.
    let Some(expected) = url_host(&ctx.oauth.public_url) else {
        return false;
    };
    let named = |value: Option<&HeaderValue>| {
        value.and_then(|v| v.to_str().ok()).is_some_and(|v| {
            v.split(',')
                .next()
                .unwrap_or("")
                .trim()
                .eq_ignore_ascii_case(&expected)
        })
    };
    let host = headers.get(header::HOST);
    let forwarded = headers.get("x-forwarded-host");
    if host.is_none() && forwarded.is_none() {
        return true;
    }
    named(host) || named(forwarded)
}

fn url_host(public_url: &str) -> Option<String> {
    let rest = public_url.split_once("://")?.1;
    Some(rest.trim_end_matches('/').to_string())
}

fn unauthorized(ctx: &McpContext, error: Option<&str>) -> Response {
    let mut response = (
        StatusCode::UNAUTHORIZED,
        Json(json!({"error": "unauthorized"})),
    )
        .into_response();
    if let Ok(value) = HeaderValue::from_str(&ctx.oauth.challenge(error, None)) {
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, value);
    }
    no_store(response)
}

async fn mcp_post(
    State(state): State<AppState>,
    Mcp(ctx): Mcp,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let ctx = &*ctx;
    if !origin_allowed(ctx, &headers) || !host_allowed(ctx, &headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    // The token is read from the header only; one in the query string is never looked at.
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            v.split_once(' ')
                .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
        })
        .map(|(_, token)| token.trim().to_string());
    let Some(token) = bearer else {
        return unauthorized(ctx, None);
    };
    let principal = match oauth::server::authenticate_bearer(&state.pool, &ctx.oauth, &token).await
    {
        Ok(Some(principal)) => principal,
        Ok(None) => return unauthorized(ctx, Some("invalid_token")),
        Err(error) => {
            tracing::error!(?error, "mcp bearer check failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    if body.len() > MAX_MCP_BODY_BYTES {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    }
    let value: Value = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(_) => {
            return no_store(
                (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"jsonrpc": "2.0", "id": null,
                                "error": {"code": -32700, "message": "Parse error"}})),
                )
                    .into_response(),
            );
        }
    };
    let mirrored = Mirrored {
        protocol_version: header_text(&headers, "mcp-protocol-version"),
        method: header_text(&headers, "mcp-method"),
        name: header_text(&headers, "mcp-name"),
    };
    let reply = penfii_mcp::handle(ctx, &principal, &mirrored, value).await;
    let status = StatusCode::from_u16(reply.status).unwrap_or(StatusCode::OK);
    let mut response = match reply.body {
        Some(body) => (status, Json(body)).into_response(),
        None => status.into_response(),
    };
    if let Some(challenge) = reply.challenge
        && let Ok(value) = HeaderValue::from_str(&challenge)
    {
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, value);
    }
    no_store(response)
}

fn header_text(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

/// `GET`/`DELETE /mcp`: there is no GET stream and no session in this server (405).
async fn mcp_other(_: Mcp) -> Response {
    let mut response = StatusCode::METHOD_NOT_ALLOWED.into_response();
    response
        .headers_mut()
        .insert(header::ALLOW, HeaderValue::from_static("POST"));
    response
}
