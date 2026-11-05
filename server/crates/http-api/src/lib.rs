//! HTTP adapter. Converts JSON contracts into core commands.

#![forbid(unsafe_code)]

mod admin;
mod collab_ws;
mod error;
mod extract;
mod knowledge;
mod mcp_routes;
mod origin;
mod routes;
mod ticket_files;
mod ws;

use std::time::Duration;

use axum::extract::{MatchedPath, Request};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::{Router, middleware};
use http::{HeaderName, HeaderValue, Method, header};
use penfii_core::docs::collab::CollabHub;
use penfii_core::files::ObjectStore;
use penfii_core::knowledge::KnowledgeConfig;
use penfii_core::{ChangeNotice, Limits};
use sqlx::PgPool;
use tokio::sync::broadcast;
use tower_http::cors::CorsLayer;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::{DefaultOnResponse, TraceLayer};
use tracing::Level;

use crate::error::ApiError;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub limits: Limits,
    pub cookie_secure: bool,
    pub changes: broadcast::Sender<ChangeNotice>,
    pub objects: Option<ObjectStore>,
    pub search: Option<penfii_core::search::SearchEngine>,
    pub knowledge: KnowledgeConfig,
    pub collab: Option<std::sync::Arc<CollabHub>>,
    /// Browser origins that may open the co-editing socket besides the server's own (`PENFII_ALLOWED_ORIGINS`).
    pub allowed_origins: Vec<String>,
    /// External sign-in providers this instance offers.
    pub providers: std::sync::Arc<penfii_core::identity::provider::AuthProviders>,
    /// MCP Authorization Server and Resource Server; `None` while `PENFII_MCP_ENABLED` is off.
    pub mcp: Option<std::sync::Arc<penfii_mcp::McpContext>>,
}

impl AppState {
    /// Tells open sockets (and the consumers sharing the broadcast) what changed. Nobody listening
    /// is not an error.
    fn publish(&self, changes: impl IntoIterator<Item = ChangeNotice>) {
        for change in changes {
            let _ = self.changes.send(change);
        }
    }
}

/// Fails when `PENFII_TRUSTED_PROXIES` is malformed. core-api calls it before it listens.
pub fn validate_trusted_proxies() -> Result<(), String> {
    extract::validate_trusted_proxies()
}

/// What the router is built from. [`RouterConfig::new`] takes what every deployment has; the rest
/// starts off and is set with struct update syntax:
///
/// ```text
/// RouterConfig { objects: Some(store), ..RouterConfig::new(pool, limits, false) }
/// ```
pub struct RouterConfig {
    pub pool: PgPool,
    pub limits: Limits,
    pub cookie_secure: bool,
    pub objects: Option<ObjectStore>,
    pub search: Option<penfii_core::search::SearchEngine>,
    /// Lets the process share one broadcast with the event consumers, so an Inbox item written by
    /// a consumer reaches the connected client. Without it the router makes its own.
    pub changes: Option<broadcast::Sender<ChangeNotice>>,
    pub knowledge: KnowledgeConfig,
    pub collab: Option<std::sync::Arc<CollabHub>>,
    pub providers: penfii_core::identity::provider::AuthProviders,
    /// Turns the MCP Authorization Server, `/mcp` and the Workspace MCP Policy API on; without it
    /// those routes answer 404.
    pub oauth: Option<penfii_core::oauth::OAuthConfig>,
}

impl RouterConfig {
    pub fn new(pool: PgPool, limits: Limits, cookie_secure: bool) -> Self {
        Self {
            pool,
            limits,
            cookie_secure,
            objects: None,
            search: None,
            changes: None,
            knowledge: KnowledgeConfig::default(),
            collab: None,
            providers: penfii_core::identity::provider::AuthProviders::default(),
            oauth: None,
        }
    }
}

pub fn router(
    pool: PgPool,
    limits: Limits,
    cookie_secure: bool,
    objects: Option<ObjectStore>,
) -> Router {
    router_with(pool, limits, cookie_secure, objects, None)
}

pub fn router_with(
    pool: PgPool,
    limits: Limits,
    cookie_secure: bool,
    objects: Option<ObjectStore>,
    search: Option<penfii_core::search::SearchEngine>,
) -> Router {
    router_from(RouterConfig {
        objects,
        search,
        ..RouterConfig::new(pool, limits, cookie_secure)
    })
}

/// The full router.
pub fn router_from(config: RouterConfig) -> Router {
    let RouterConfig {
        pool,
        limits,
        cookie_secure,
        objects,
        search,
        changes,
        knowledge,
        collab,
        providers,
        oauth,
    } = config;
    let changes = changes.unwrap_or_else(|| broadcast::channel(256).0);
    let allowed_origins = origin::allowed_origins_from_env();
    let mcp = oauth.map(|config| {
        std::sync::Arc::new(penfii_mcp::McpContext::new(
            pool.clone(),
            config,
            search.clone(),
            changes.clone(),
        ))
    });
    let state = AppState {
        pool,
        limits,
        cookie_secure,
        changes,
        objects,
        search,
        knowledge,
        collab,
        allowed_origins: allowed_origins.clone(),
        providers: std::sync::Arc::new(providers),
        mcp,
    };
    api_routes()
        .layer(middleware::from_fn(csrf))
        .layer(cors_layer(&allowed_origins))
        // What every response from core-api tells the browser, unless the handler said it already.
        // core-api answers with JSON, files and the odd error page, never with the web app, so the
        // policy is the strictest one: nothing may load, and no page may frame it. `nosniff` stops a
        // browser from treating a download as something other than its declared type. The web
        // app's own headers (the real CSP) are set by the web server that serves it, see
        // `docker/web/Caddyfile`.
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("default-src 'none'; frame-ancestors 'none'"),
        ))
        // One `info` line per request: method, the matched route template (never the raw URI, whose
        // ids, query strings and OAuth codes stay out of the log), status and latency; 5xx answers
        // log at `error`. No header is recorded.
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &Request| {
                    let route = request
                        .extensions()
                        .get::<MatchedPath>()
                        .map(MatchedPath::as_str);
                    tracing::info_span!("request", method = %request.method(), route)
                })
                .on_request(())
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        )
        .with_state(state)
}

/// The route table. Each area lists its own routes next to its handlers; this is the index of them.
fn api_routes() -> Router<AppState> {
    Router::new()
        .merge(routes::routes())
        .merge(admin::routes())
        .merge(ticket_files::routes())
        .merge(knowledge::routes())
        .merge(mcp_routes::routes())
        .merge(ws::routes())
        .merge(collab_ws::routes())
}

fn cors_layer(origins: &[String]) -> CorsLayer {
    let list: Vec<HeaderValue> = origins
        .iter()
        .filter_map(|origin| HeaderValue::from_str(origin).ok())
        .collect();
    CorsLayer::new()
        .allow_origin(list)
        .allow_credentials(true)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([
            header::CONTENT_TYPE,
            header::COOKIE,
            HeaderName::from_static("x-penfii-client"),
            HeaderName::from_static("idempotency-key"),
        ])
        .max_age(Duration::from_secs(600))
}

/// Paths with no cookie that authenticate by token or PKCE; they never read the session cookie
///, so the cookie-bound CSRF header rule does not apply to them.
fn csrf_exempt(path: &str) -> bool {
    matches!(
        path,
        "/oauth/token" | "/oauth/revoke" | "/oauth/register" | "/mcp"
    )
}

async fn csrf(request: Request, next: Next) -> Response {
    if !csrf_exempt(request.uri().path())
        && matches!(
            *request.method(),
            Method::POST | Method::PUT | Method::PATCH | Method::DELETE
        )
    {
        let header_ok = request
            .headers()
            .get("x-penfii-client")
            .and_then(|value| value.to_str().ok())
            == Some("1");
        if !header_ok {
            return ApiError::csrf().into_response();
        }
    }
    next.run(request).await
}
